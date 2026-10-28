//! Subsonic/OpenSubsonic compatibility façade.

use std::collections::BTreeMap;

use axum::{
    body::to_bytes,
    extract::{Path, Request, State},
    http::{header, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use md5::{Digest, Md5};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::{
    auth::security,
    media::{MediaError, OutputFormat, StreamQuery},
    services::{
        AlbumItem, AlbumListQuery, AlbumOrder, ArtistItem, ArtistSummary, BrowsePage, PlaylistItem,
        ServiceError, SongItem,
    },
    storage::database::AccountRole,
    AppState,
};

mod admin;
mod auth;
mod browse;
mod errors;
mod legacy;
mod lyrics;
mod media;
mod nodes;
mod playlists;
mod podcasts;
mod protocol;
mod radio;
mod shares;
mod smart_playlists;
mod userdata;

use admin::*;
use auth::*;
use browse::*;
use errors::*;
use legacy::*;
use lyrics::*;
use media::*;
use nodes::*;
use playlists::*;
use podcasts::*;
use protocol::*;
use radio::*;
use shares::*;
use smart_playlists::*;
use userdata::*;

const SUBSONIC_VERSION: &str = "1.16.1";

const XMLNS: &str = "http://subsonic.org/restapi";

/// The most a `formPost` body may hold (RFC-022 decision 7).
///
/// Sized for the largest request the domain accepts: a playlist of
/// [`MAX_PLAYLIST_TRACKS`](crate::services::MAX_PLAYLIST_TRACKS) identifiers in
/// one call. Sixty-four bytes an identifier covers the longest pair the façade
/// takes, `songIdToAdd=<uuid>&` at forty-nine, and UUIDs never need percent
/// encoding; the sixty-four kibibytes on top, the whole of the old limit, are
/// for the credentials, a name, a comment and whatever else rides beside them.
///
/// Derived rather than written down, like the playlist import limit: raising
/// the playlist ceiling must not quietly start refusing the requests it has
/// just permitted. The old fixed 64 KiB stopped `createPlaylist` short of
/// 1,490 tracks and `updatePlaylist` short of 1,338, against a ceiling of ten
/// thousand.
/// Still a bound: one request is buffered once, up to this, and a byte more is
/// refused with error 10.
pub const MAX_FORM_BYTES: usize = crate::services::MAX_PLAYLIST_TRACKS * 64 + 64 * 1024;

/// How many album-less tracks a folder listing will carry.
///
/// `getMusicDirectory` takes no offset, so a folder cannot be paged and the
/// only bound available is a ceiling. It sits far above `MAX_BROWSE_LIMIT`
/// because reaching it costs a client the tracks beyond it — the browse limit
/// governs a listing the client can ask more of, this one governs a listing it
/// cannot. A folder that reaches it is logged.
const MAX_DIRECTORY_SONGS: i64 = 2_000;

#[derive(Debug, Clone)]
struct Principal {
    id: Uuid,
    username: String,
    role: AccountRole,
}

#[derive(Debug, Default)]
struct Params(Vec<(String, String)>);

#[derive(Debug, Clone)]
struct Node {
    name: String,
    attrs: BTreeMap<String, Value>,
    children: Vec<Node>,
    text: Option<String>,
}

impl Node {
    fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            attrs: BTreeMap::new(),
            children: Vec::new(),
            text: None,
        }
    }

    /// The answer that carries no payload at all.
    ///
    /// Distinct from an *empty* container: some methods have no
    /// schema-conforming shape to answer with when they have nothing to say,
    /// and for those the envelope alone is the answer. An element name can
    /// never be empty, so the empty name is a sentinel no real node collides
    /// with. See `empty_success_method` for the static half of the
    /// same rule — that list is per method, this is per answer.
    fn nothing() -> Self {
        Self::new(String::new())
    }

    fn is_nothing(&self) -> bool {
        self.name.is_empty()
    }

    fn attr(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.attrs.insert(key.into(), value.into());
        self
    }

    fn maybe_attr(mut self, key: &str, value: Option<impl Into<Value>>) -> Self {
        if let Some(value) = value {
            self.attrs.insert(key.to_owned(), value.into());
        }
        self
    }

    fn child(mut self, child: Node) -> Self {
        self.children.push(child);
        self
    }

    fn text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    fn renamed(mut self, name: &'static str) -> Self {
        self.name = name.to_owned();
        self
    }

    fn without(mut self, key: &str) -> Self {
        self.attrs.remove(key);
        self
    }

    fn children(mut self, children: impl IntoIterator<Item = Node>) -> Self {
        self.children.extend(children);
        self
    }
}

/// A Subsonic protocol failure.
///
/// The transport status is deliberately not carried here. OpenSubsonic answers
/// every request it could parse with HTTP 200 and reports the failure in the
/// body, so a client reading `error/code` sees the same outcome whatever the
/// transport did. Answering 401 or 404 instead let proxies and HTTP-level
/// client error handling discard the body before the Subsonic layer read it.
#[derive(Debug)]
struct ProtocolError {
    code: i64,
    message: &'static str,
}

pub fn router(state: AppState) -> Router {
    // `/rest` carries none of the security headers: the façade's wire
    // behaviour is frozen (RFC-019 decision 2). The public shares, which do
    // carry them, are served by `crate::http::public_share` since RFC-020.
    Router::new()
        .route("/rest/{method}", get(handle).post(handle))
        .with_state(state)
}

pub async fn handle(
    State(state): State<AppState>,
    Path(raw_method): Path<String>,
    request: Request,
) -> Response {
    let request_method = request.method().clone();
    let query = request.uri().query().unwrap_or_default().to_owned();
    // Read before the body is: a POST's request is consumed below.
    let address = crate::auth::sign_in::request_bucket(
        request.extensions(),
        request.headers(),
        &state.trusted_proxies,
    );
    let range = request
        .headers()
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    // Format negotiation has to survive a failure, and under the `formPost`
    // extension `f=json` arrives in the body rather than the query string.
    // Collecting the parameters here, outside the fallible path, is what lets a
    // POST that fails to authenticate still answer in the format it asked for
    // instead of falling back to XML. The wire survives the same way: a client
    // naming itself in the query is answered as itself even when its body
    // cannot be read.
    let mut wants_json = false;
    let mut wire = Wire::OpenSubsonic;
    let params = match parse_pairs(&query) {
        Ok(mut params) => {
            wants_json = json_requested(&params);
            wire = Wire::for_request(&state, &params);
            if request_method == Method::POST {
                match form_params(request).await {
                    Ok(body) => {
                        params.0.extend(body.0);
                        wants_json = json_requested(&params);
                        wire = Wire::for_request(&state, &params);
                        Ok(params)
                    }
                    Err(error) => Err(error),
                }
            } else {
                Ok(params)
            }
        }
        Err(error) => Err(error),
    };

    let outcome = match params {
        Ok(params) => {
            handle_inner(
                &state,
                &raw_method,
                &request_method,
                &params,
                range.as_deref(),
                &address,
            )
            .await
        }
        Err(error) => Err(error),
    };
    match outcome {
        Ok(response) => response,
        // A protocol failure is still an HTTP success: the Subsonic contract
        // puts the outcome in the body, never in the status line. An error
        // envelope carries no media item, so its body is the same on either
        // wire; its media type is not, and an old client in place of a stream
        // tells the error from audio by that type alone.
        Err(error) => render_protocol(error_node(error.code, error.message), wants_json, wire),
    }
}

/// Parameters carried in a POST body, as the `formPost` extension allows in
/// place of a query string too long for a URL.
async fn form_params(request: Request) -> Result<Params, ProtocolError> {
    // A media type is case-insensitive and may carry parameters, so the type is
    // compared on its own rather than as a prefix of the raw header value:
    // `Application/X-WWW-Form-Urlencoded; charset=UTF-8` is a conformant way to
    // say the same thing, and `application/x-www-form-urlencodedish` is not.
    let media_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim();
    if !media_type.eq_ignore_ascii_case("application/x-www-form-urlencoded") {
        return Err(invalid("POST requires application/x-www-form-urlencoded"));
    }
    let body = to_bytes(request.into_body(), MAX_FORM_BYTES)
        .await
        .map_err(|_| invalid("Invalid form body"))?;
    parse_pairs(std::str::from_utf8(&body).map_err(|_| invalid("Invalid form body"))?)
}

fn json_requested(params: &Params) -> bool {
    params.first("f").is_some_and(|value| value == "json")
}

async fn handle_inner(
    state: &AppState,
    raw_method: &str,
    request_method: &Method,
    params: &Params,
    range: Option<&str>,
    address: &str,
) -> Result<Response, ProtocolError> {
    let method = raw_method.strip_suffix(".view").unwrap_or(raw_method);
    let wants_json = json_requested(params);
    if is_symfonium_discovery_probe(method, request_method, params) {
        return Ok(render_protocol(ok_node(), wants_json, Wire::OpenSubsonic));
    }
    let principal = authenticate(state, params, address).await?;

    if matches!(method, "stream" | "download") {
        return media_response(state, &principal, params, method == "download", range).await;
    }
    if method == "getCoverArt" {
        return cover_art_response(state, &principal, params).await;
    }

    let payload = dispatch(state, &principal, method, params).await?;
    let root = if method == "ping" || empty_success_method(method) || payload.is_nothing() {
        ok_node()
    } else {
        ok_node().child(payload)
    };
    Ok(render_protocol(
        root,
        wants_json,
        Wire::for_request(state, params),
    ))
}

fn is_symfonium_discovery_probe(method: &str, request_method: &Method, params: &Params) -> bool {
    method == "ping"
        && request_method == Method::GET
        && params.all("c") == ["Symfonium"]
        && params.all("u") == ["test"]
        && params.all("p") == ["test"]
        && params.all("apiKey").is_empty()
        && params.all("t").is_empty()
        && params.all("s").is_empty()
}

async fn dispatch(
    state: &AppState,
    principal: &Principal,
    method: &str,
    params: &Params,
) -> Result<Node, ProtocolError> {
    match method {
        "ping" => Ok(Node::new("ping")),
        "getLicense" => Ok(Node::new("license")
            .attr("valid", true)
            .attr("email", "")
            .attr("licenseExpires", "2099-12-31T23:59:59Z")),
        "getOpenSubsonicExtensions" => Ok(open_subsonic_extensions()),
        // The other half of the apiKeyAuthentication extension: a client holding
        // a key has no other way to learn which account it speaks for.
        // Advertising the extension without serving this told clients a lie.
        "tokenInfo" => Ok(Node::new("tokenInfo").attr("username", principal.username.clone())),
        // Playback positions, one per account and track. Symfonium asks for
        // them during its initial sync, and they are now read from and written
        // to the catalogue rather than answered with an empty container.
        "getBookmarks" => bookmarks(state, principal).await,
        "createBookmark" => create_bookmark(state, principal, params).await,
        "deleteBookmark" => delete_bookmark(state, principal, params).await,
        // An artist's most played tracks, counted from this account's own
        // listening. Not a recommendation: the reference server asks a remote
        // service what is popular, and this answers what was played here.
        "getTopSongs" => top_songs(state, principal, params).await,
        // A draw from the artist and their neighbours, read from the
        // enrichment cache (RFC-011). With no provider configured — the
        // default — still the standard empty container.
        "getSimilarSongs" => similar_songs(state, principal, params, "similarSongs").await,
        "getSimilarSongs2" => similar_songs(state, principal, params, "similarSongs2").await,
        // The last empty container to be filled. It is per account, which the
        // reference server's list is not — see `src/subsonic/radio.rs` for that
        // deviation and the one about `homePageUrl`'s spelling.
        "getInternetRadioStations"
        | "createInternetRadioStation"
        | "updateInternetRadioStation"
        | "deleteInternetRadioStation" => radio_stations(state, principal, method, params).await,
        // No avatars are stored, so the account genuinely has none. Code 70
        // says that; code 0 would blame the method instead of the data.
        "getAvatar" => Err(not_found()),
        "startScan" => start_scan(state, principal).await,
        "getScanStatus" => scan_status(state, principal).await,
        "getMusicFolders" => {
            let folders = state
                .services
                .music_folders(principal.id, &[])
                .await
                .map_err(internal)?;
            Ok(
                Node::new("musicFolders").children(folders.into_iter().map(|folder| {
                    Node::new("musicFolder")
                        .attr("id", folder.id.to_string())
                        .attr("name", folder.name)
                })),
            )
        }
        "getIndexes" => indexes(state, principal, params, false).await,
        "getArtists" => indexes(state, principal, params, true).await,
        "getArtist" => get_artist(state, principal, params).await,
        // DSub requests artist information as soon as an artist page opens.
        // Answered from the enrichment cache when an operator switched it on
        // (RFC-011), and the empty standard container otherwise, so an
        // optional capability never becomes a blocking client error. The
        // artist is resolved tenant-side either way.
        "getArtistInfo" => artist_info(state, principal, params, "artistInfo").await,
        "getArtistInfo2" => artist_info(state, principal, params, "artistInfo2").await,
        // Feishin and Symfonium call these as soon as an album page opens.
        // Album notes are not enriched, so the standard container with only
        // what the catalogue holds is the honest answer — and it still
        // resolves the album tenant-side, so a foreign id is
        // indistinguishable from a missing one.
        "getAlbumInfo" => album_info(state, principal, params, "albumInfo").await,
        "getAlbumInfo2" => album_info(state, principal, params, "albumInfo2").await,
        "getAlbum" => get_album(state, principal, params).await,
        "getSong" => get_song(state, principal, params).await,
        "getLyrics" => get_lyrics(state, principal, params).await,
        "getLyricsBySongId" => get_lyrics_by_song_id(state, principal, params).await,
        "getGenres" => genres(state, principal, params).await,
        "getMusicDirectory" => music_directory(state, principal, params).await,
        "getAlbumList2" => album_list(state, principal, params).await,
        // Older clients such as DSub still use the pre-ID3 endpoint. The
        // payload is identical for our UUID catalogue; only the container
        // name differs from getAlbumList2.
        "getAlbumList" => album_list(state, principal, params)
            .await
            .map(|node| node.renamed("albumList")),
        "getRandomSongs" => random_songs(state, principal, params).await,
        "getSongsByGenre" => songs_by_genre(state, principal, params).await,
        "search3" => search(state, principal, params).await,
        "search2" => search(state, principal, params)
            .await
            .map(|node| node.renamed("searchResult2")),
        "getPlaylists" => playlists(state, principal, params).await,
        "getPlaylist" => get_playlist(state, principal, params).await,
        "createPlaylist" => create_playlist(state, principal, params).await,
        "updatePlaylist" => update_playlist(state, principal, params).await,
        "deletePlaylist" => delete_playlist(state, principal, params).await,
        "star" => set_star(state, principal, params, true).await,
        "unstar" => set_star(state, principal, params, false).await,
        "getStarred2" => starred(state, principal, params).await,
        // Browse-by-folder clients such as DSub and Ultrasonic still call the
        // pre-ID3 method. Same payload for a UUID catalogue; only the
        // container differs, exactly as for getAlbumList.
        "getStarred" => starred(state, principal, params)
            .await
            .map(|node| node.renamed("starred")),
        "setRating" => set_rating(state, principal, params).await,
        "scrobble" => scrobble(state, principal, params).await,
        "getNowPlaying" => now_playing(state, principal).await,
        "getPlayQueue" => get_queue(state, principal).await,
        "savePlayQueue" => save_queue(state, principal, params).await,
        "getShares" | "createShare" | "updateShare" | "deleteShare" => {
            shares(state, principal, method, params).await
        }
        "getUser" | "getUsers" | "createUser" | "updateUser" | "deleteUser" | "changePassword" => {
            admin(state, principal, method, params).await
        }
        // One list per instance, managed by `podcastRole` (RFC-025). These
        // answered "not implemented" until then.
        "getPodcasts"
        | "getNewestPodcasts"
        | "getPodcastEpisode"
        | "refreshPodcasts"
        | "createPodcastChannel"
        | "deletePodcastChannel"
        | "deletePodcastEpisode"
        | "downloadPodcastEpisode" => podcasts(state, principal, method, params).await,
        _ => Err(ProtocolError {
            code: 0,
            message: "Requested method is not implemented",
        }),
    }
}

impl Params {
    fn first(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }
    fn all(&self, key: &str) -> Vec<&str> {
        self.0
            .iter()
            .filter(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
            .collect()
    }
    fn uuid(&self, key: &str) -> Result<Uuid, ProtocolError> {
        self.first(key)
            .ok_or_else(missing)?
            .parse()
            .map_err(|_| invalid("Invalid UUID"))
    }
    fn uuid_optional(&self, key: &str) -> Result<Option<Uuid>, ProtocolError> {
        self.first(key)
            .map(|value| value.parse().map_err(|_| invalid("Invalid UUID")))
            .transpose()
    }
    fn uuids(&self, key: &str) -> Result<Vec<Uuid>, ProtocolError> {
        self.all(key)
            .into_iter()
            .map(|value| value.parse().map_err(|_| invalid("Invalid UUID")))
            .collect()
    }
    fn usizes(&self, key: &str) -> Result<Vec<usize>, ProtocolError> {
        self.all(key)
            .into_iter()
            .map(|value| value.parse().map_err(|_| invalid("Invalid number")))
            .collect()
    }
    fn i64(&self, key: &str) -> Result<i64, ProtocolError> {
        self.first(key)
            .ok_or_else(missing)?
            .parse()
            .map_err(|_| invalid("Invalid number"))
    }
    fn i64_optional(&self, key: &str) -> Result<Option<i64>, ProtocolError> {
        self.first(key)
            .map(|value| value.parse().map_err(|_| invalid("Invalid number")))
            .transpose()
    }
    fn i64_or(&self, key: &str, default: i64) -> Result<i64, ProtocolError> {
        self.first(key)
            .map(|value| value.parse().map_err(|_| invalid("Invalid number")))
            .transpose()
            .map(|value| value.unwrap_or(default))
    }
    fn usize_or(&self, key: &str, default: usize, max: usize) -> Result<usize, ProtocolError> {
        let value = self
            .first(key)
            .map(|value| value.parse().map_err(|_| invalid("Invalid number")))
            .transpose()?
            .unwrap_or(default);
        Ok(value.min(max))
    }
    fn bool_optional(&self, key: &str) -> Result<Option<bool>, ProtocolError> {
        self.first(key)
            .map(|value| match value {
                "true" | "1" => Ok(true),
                "false" | "0" => Ok(false),
                _ => Err(invalid("Invalid boolean")),
            })
            .transpose()
    }
}

/// What a rendered node is, told from the fields it carries rather than from
/// its element name — which `getMusicDirectory` collapses to `child` for all
/// three.
enum EntryKind {
    Artist,
    Album,
    Song,
}
