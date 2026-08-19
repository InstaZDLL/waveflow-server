//! Subsonic/OpenSubsonic compatibility façade.

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{Mutex as StdMutex, OnceLock},
    time::{Duration, Instant},
};

use axum::{
    body::to_bytes,
    extract::{Path, Query, Request, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use md5::{Digest, Md5};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::{
    database::AccountRole,
    media::{MediaError, OutputFormat, StreamQuery},
    security,
    services::{
        AlbumItem, AlbumListQuery, AlbumOrder, ArtistItem, BrowsePage, PlaylistItem, ServiceError,
        SongItem,
    },
    AppState,
};

const SUBSONIC_VERSION: &str = "1.16.1";
const XMLNS: &str = "http://subsonic.org/restapi";
const MAX_FORM_BYTES: usize = 64 * 1024;
const AUTH_ATTEMPTS_PER_MINUTE: usize = 20;
const MAX_AUTH_RATE_KEYS: usize = 10_000;

static AUTH_WINDOWS: OnceLock<StdMutex<HashMap<String, VecDeque<Instant>>>> = OnceLock::new();

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
    Router::new()
        .route("/rest/{method}", get(handle).post(handle))
        .route(
            "/share/{token}/tracks/{track_id}/stream",
            get(public_share_stream),
        )
        .route("/share/{token}", get(public_share))
        .with_state(state)
}

async fn public_share(State(state): State<AppState>, Path(token): Path<String>) -> Response {
    match state.services.public_share(&token).await {
        Ok(share) => {
            let tracks = share
                .songs
                .iter()
                .map(|song| {
                    let mut value = serde_json::to_value(song).expect("song serialization");
                    if let Value::Object(object) = &mut value {
                        object.insert(
                            "streamUrl".into(),
                            Value::String(external_url(
                                state.public_url.as_deref(),
                                &format!("/share/{token}/tracks/{}/stream", song.id),
                            )),
                        );
                    }
                    value
                })
                .collect::<Vec<_>>();
            (
                [(header::CACHE_CONTROL, "no-store")],
                axum::Json(serde_json::json!({
                    "id": share.id,
                    "description": share.description,
                    "expiresAt": share.expires_at,
                    "visitCount": share.visit_count,
                    "tracks": tracks,
                })),
            )
                .into_response()
        }
        Err(ServiceError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::error!(error = %error, "public share lookup failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn public_share_stream(
    State(state): State<AppState>,
    Path((token, track_id)): Path<(String, Uuid)>,
    Query(query): Query<StreamQuery>,
    headers: HeaderMap,
) -> Response {
    let share = match state.services.public_share(&token).await {
        Ok(share) => share,
        Err(ServiceError::NotFound) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::error!(error = %error, "public share stream lookup failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if !share.songs.iter().any(|song| song.id == track_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let track = match state
        .db
        .stream_track_for_user(share.owner_id, track_id)
        .await
    {
        Ok(Some(track)) => track,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::error!(error = %error, "public share media lookup failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());
    match state.media.serve(share.owner_id, track, query, range).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

pub async fn handle(
    State(state): State<AppState>,
    Path(raw_method): Path<String>,
    request: Request,
) -> Response {
    let request_method = request.method().clone();
    let query = request.uri().query().unwrap_or_default().to_owned();
    let range = request
        .headers()
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    // Format negotiation has to survive a failure, and under the `formPost`
    // extension `f=json` arrives in the body rather than the query string.
    // Collecting the parameters here, outside the fallible path, is what lets a
    // POST that fails to authenticate still answer in the format it asked for
    // instead of falling back to XML.
    let mut wants_json = false;
    let params = match parse_pairs(&query) {
        Ok(mut params) => {
            wants_json = json_requested(&params);
            if request_method == Method::POST {
                match form_params(request).await {
                    Ok(body) => {
                        params.0.extend(body.0);
                        wants_json = json_requested(&params);
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
            )
            .await
        }
        Err(error) => Err(error),
    };
    match outcome {
        Ok(response) => response,
        // A protocol failure is still an HTTP success: the Subsonic contract
        // puts the outcome in the body, never in the status line.
        Err(error) => render_protocol(error_node(error.code, error.message), wants_json),
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
) -> Result<Response, ProtocolError> {
    let method = raw_method.strip_suffix(".view").unwrap_or(raw_method);
    let wants_json = json_requested(params);
    if is_symfonium_discovery_probe(method, request_method, params) {
        return Ok(render_protocol(ok_node(), wants_json));
    }
    let principal = authenticate(state, params).await?;

    if matches!(method, "stream" | "download") {
        return media_response(state, &principal, params, method == "download", range).await;
    }
    if method == "getCoverArt" {
        return cover_art_response(state, &principal, params).await;
    }

    let payload = dispatch(state, &principal, method, params).await?;
    let root = if method == "ping" || empty_success_method(method) {
        ok_node()
    } else {
        ok_node().child(payload)
    };
    Ok(render_protocol(root, wants_json))
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
        // Recommendation and radio surfaces WaveFlow does not compute. The
        // standard empty container is the honest answer and, unlike the
        // not-implemented error, does not read to a client as a broken
        // server on a page it opens by default.
        "getTopSongs" => Ok(Node::new("topSongs")),
        "getSimilarSongs" => Ok(Node::new("similarSongs")),
        "getSimilarSongs2" => Ok(Node::new("similarSongs2")),
        "getInternetRadioStations" => Ok(Node::new("internetRadioStations")),
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
        // WaveFlow does not enrich biographies yet, but a successful empty
        // standard container avoids turning an optional capability into a
        // blocking client error. The artist is still resolved tenant-side.
        "getArtistInfo" => artist_info(state, principal, params, "artistInfo").await,
        "getArtistInfo2" => artist_info(state, principal, params, "artistInfo2").await,
        // Feishin and Symfonium call these as soon as an album page opens. As
        // with getArtistInfo, WaveFlow enriches nothing yet, so the standard
        // empty container is the honest answer — and it still resolves the
        // album tenant-side, so a foreign id is indistinguishable from a
        // missing one.
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
        "getPlaylists" => playlists(state, principal).await,
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
        _ => Err(ProtocolError {
            code: 0,
            message: "Requested method is not implemented",
        }),
    }
}

async fn authenticate(state: &AppState, params: &Params) -> Result<Principal, ProtocolError> {
    let rate_key = params
        .first("apiKey")
        .or_else(|| params.first("u"))
        .unwrap_or("missing");
    let rate_hash = hex::encode(security::token_hash(rate_key));
    if auth_rate_limited(&rate_hash) {
        return Err(ProtocolError {
            code: 40,
            message: "Wrong username or password",
        });
    }

    let credential = if let Some(api_key) = params.first("apiKey") {
        state
            .services
            .credential_by_api_key(api_key)
            .await
            .map_err(internal)?
            .ok_or_else(|| {
                record_auth_failure(&rate_hash);
                auth_error()
            })?
    } else {
        let username = params.first("u").ok_or_else(missing)?;
        state
            .services
            .credential_by_username(username)
            .await
            .map_err(internal)?
            .ok_or_else(|| {
                record_auth_failure(&rate_hash);
                auth_error()
            })?
    };
    let password = state
        .services
        .decrypt_subsonic_password(&credential)
        .map_err(internal)?;
    if params.first("apiKey").is_none() {
        let valid = if let (Some(token), Some(salt)) = (params.first("t"), params.first("s")) {
            let mut digest = Md5::new();
            digest.update(&password);
            digest.update(salt.as_bytes());
            let expected = hex::encode(digest.finalize());
            security::constant_time_bytes_eq(token.as_bytes(), expected.as_bytes())
        } else if let Some(provided) = params.first("p") {
            let decoded = match provided.strip_prefix("enc:").map(hex::decode).transpose() {
                Ok(value) => value.unwrap_or_else(|| provided.as_bytes().to_vec()),
                Err(_) => {
                    record_auth_failure(&rate_hash);
                    return Err(auth_error());
                }
            };
            security::constant_time_bytes_eq(&decoded, &password)
        } else {
            false
        };
        if !valid {
            record_auth_failure(&rate_hash);
            return Err(auth_error());
        }
    }
    clear_auth_failures(&rate_hash);
    Ok(Principal {
        id: credential.account.id,
        username: credential.account.username,
        role: credential.account.role,
    })
}

fn auth_rate_limited(key: &str) -> bool {
    let now = Instant::now();
    let windows = AUTH_WINDOWS.get_or_init(|| StdMutex::new(HashMap::new()));
    let Ok(mut windows) = windows.lock() else {
        return false;
    };
    prune_auth_windows(&mut windows, now);
    let attempts = windows.entry(key.to_owned()).or_default();
    attempts.len() >= AUTH_ATTEMPTS_PER_MINUTE
}

fn record_auth_failure(key: &str) {
    let windows = AUTH_WINDOWS.get_or_init(|| StdMutex::new(HashMap::new()));
    if let Ok(mut windows) = windows.lock() {
        let now = Instant::now();
        prune_auth_windows(&mut windows, now);
        if !windows.contains_key(key) && windows.len() >= MAX_AUTH_RATE_KEYS {
            if let Some(oldest) = windows
                .iter()
                .min_by_key(|(_, attempts)| attempts.back().copied())
                .map(|(key, _)| key.clone())
            {
                windows.remove(&oldest);
            }
        }
        windows.entry(key.to_owned()).or_default().push_back(now);
    }
}

fn prune_auth_windows(windows: &mut HashMap<String, VecDeque<Instant>>, now: Instant) {
    windows.retain(|_, attempts| {
        while attempts
            .front()
            .is_some_and(|time| now.duration_since(*time) >= Duration::from_secs(60))
        {
            attempts.pop_front();
        }
        !attempts.is_empty()
    });
}

fn clear_auth_failures(key: &str) {
    let windows = AUTH_WINDOWS.get_or_init(|| StdMutex::new(HashMap::new()));
    if let Ok(mut windows) = windows.lock() {
        windows.remove(key);
    }
}

/// Folders, artists and albums for the requested libraries.
///
/// Preferred over [`crate::services::DomainServices::catalog_snapshot`]
/// wherever the answer does not contain tracks: the track read is the
/// expensive third of a snapshot, and since the OpenSubsonic fields landed it
/// carries two relation loads of its own.
async fn overview(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<crate::services::CatalogOverview, ProtocolError> {
    let folders = params.uuids("musicFolderId")?;
    state
        .services
        .catalog_overview(principal.id, &folders)
        .await
        .map_err(internal)
}

/// Rescans every library the account can reach.
///
/// Subsonic has no library parameter here, so this fans out. Both the
/// authorization and the queuing live in
/// [`crate::services::DomainServices::start_visible_scans`], so this facade
/// and the native per-library endpoint cannot disagree about who may scan
/// what.
async fn start_scan(state: &AppState, principal: &Principal) -> Result<Node, ProtocolError> {
    state
        .services
        .start_visible_scans(principal.id)
        .await
        .map_err(service_protocol)?;
    // The protocol answers a start with the resulting status, so a client
    // that only calls startScan still learns whether anything is running.
    scan_status(state, principal).await
}

/// `count` is the number of available tracks *this* account can reach, not
/// what the instance holds: the rest of the facade never reports a total that
/// includes another tenant's catalogue, and this is no exception.
async fn scan_status(state: &AppState, principal: &Principal) -> Result<Node, ProtocolError> {
    let (scanning, count) = state
        .db
        .scan_progress_for_user(principal.id)
        .await
        .map_err(internal)?;
    Ok(Node::new("scanStatus")
        .attr("scanning", scanning)
        .attr("count", count))
}

async fn indexes(
    state: &AppState,
    principal: &Principal,
    params: &Params,
    id3: bool,
) -> Result<Node, ProtocolError> {
    let overview = overview(state, principal, params).await?;
    let mut groups: BTreeMap<char, Vec<ArtistItem>> = BTreeMap::new();
    for artist in overview.artists {
        let initial = artist
            .name
            .chars()
            .next()
            .filter(char::is_ascii_alphabetic)
            .map(|value| value.to_ascii_uppercase())
            .unwrap_or('#');
        groups.entry(initial).or_default().push(artist);
    }
    let root_name = if id3 { "artists" } else { "indexes" };
    Ok(Node::new(root_name)
        .attr("ignoredArticles", "The El La Les Le L'")
        .attr("lastModified", chrono::Utc::now().timestamp_millis())
        .children(groups.into_iter().map(|(letter, artists)| {
            Node::new("index")
                .attr("name", letter.to_string())
                .children(artists.into_iter().map(|artist| {
                    artist_node(
                        &artist,
                        overview
                            .albums
                            .iter()
                            .filter(|album| album.artist_id == Some(artist.id))
                            .count(),
                    )
                }))
        })))
}

async fn get_artist(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let detail = state
        .services
        .artist(principal.id, params.uuid("id")?)
        .await
        .map_err(service_protocol)?;
    // `album_count` comes from the projection rather than from the length of
    // the list below. They agree today only because `albums` is unpaginated,
    // which is an unwritten guarantee the response should not rest on.
    Ok(artist_node(&detail.artist, detail.album_count as usize)
        .children(detail.albums.iter().map(album_node)))
}

async fn artist_info(
    state: &AppState,
    principal: &Principal,
    params: &Params,
    container: &'static str,
) -> Result<Node, ProtocolError> {
    state
        .services
        .artist(principal.id, params.uuid("id")?)
        .await
        .map_err(service_protocol)?;
    Ok(Node::new(container))
}

/// `getAlbumInfo` and `getAlbumInfo2`.
///
/// WaveFlow queries no remote source, so notes and biography images stay
/// absent. The release identifier is the one part of the answer the catalogue
/// actually holds, and it is emitted when the album has one. `AlbumInfo`
/// predates the OpenSubsonic presence rule, so an album without a release id
/// omits the element rather than sending it empty.
///
/// The lookup runs first and for its refusal: it is what turns an album the
/// caller cannot reach into the same answer as one that does not exist.
async fn album_info(
    state: &AppState,
    principal: &Principal,
    params: &Params,
    container: &'static str,
) -> Result<Node, ProtocolError> {
    let album = state
        .services
        .album(principal.id, params.uuid("id")?)
        .await
        .map_err(service_protocol)?;
    Ok(Node::new(container).children(
        album
            .album
            .musicbrainz_id
            .map(|id| Node::new("musicBrainzId").text(id)),
    ))
}

async fn get_album(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let detail = state
        .services
        .album(principal.id, params.uuid("id")?)
        .await
        .map_err(service_protocol)?;
    Ok(album_node(&detail.album).children(detail.songs.iter().map(song_node)))
}

async fn get_song(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let id = params.uuid("id")?;
    let song = state
        .services
        .songs_by_ids(principal.id, &[id])
        .await
        .map_err(service_protocol)?
        .into_iter()
        .next()
        .ok_or_else(not_found)?;
    Ok(song_node(&song))
}

async fn get_lyrics(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let artist = params.first("artist");
    let title = params.first("title");
    let Some(lyrics) = state
        .services
        .lyrics_by_metadata(principal.id, artist, title)
        .await
        .map_err(service_protocol)?
    else {
        return Ok(Node::new("lyrics")
            .maybe_attr("artist", artist.map(str::to_owned))
            .maybe_attr("title", title.map(str::to_owned)));
    };
    let Some(first) = lyrics.structured_lyrics.first() else {
        return Ok(Node::new("lyrics"));
    };
    Ok(Node::new("lyrics")
        .maybe_attr("artist", first.display_artist.clone())
        .attr("title", first.display_title.clone())
        .text(
            first
                .lines
                .iter()
                .map(|line| line.value.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        ))
}

async fn get_lyrics_by_song_id(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let lyrics = state
        .services
        .lyrics(principal.id, params.uuid("id")?)
        .await
        .map_err(service_protocol)?;
    Ok(Node::new("lyricsList")
        .children(lyrics.structured_lyrics.iter().map(structured_lyrics_node)))
}

fn structured_lyrics_node(lyrics: &crate::lyrics::StructuredLyrics) -> Node {
    Node::new("structuredLyrics")
        .maybe_attr("displayArtist", lyrics.display_artist.clone())
        .attr("displayTitle", lyrics.display_title.clone())
        .attr("lang", lyrics.lang.clone())
        .attr("synced", lyrics.synced)
        .children(lyrics.lines.iter().map(|line| {
            Node::new("line")
                .maybe_attr("start", line.start)
                .text(line.value.clone())
        }))
}

/// Parameter adapter over [`crate::services::DomainServices::list_genres`].
async fn genres(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let genres = state
        .services
        .list_genres(principal.id, &params.uuids("musicFolderId")?)
        .await
        .map_err(internal)?;
    Ok(
        Node::new("genres").children(genres.into_iter().map(|genre| {
            Node::new("genre")
                .attr("songCount", genre.song_count)
                .attr("albumCount", genre.album_count)
                .text(genre.name)
        })),
    )
}

/// Renders an artist or album as a browsing entry of `getMusicDirectory`.
///
/// `musicBrainzId` is dropped on the way. On a `Child` the specification
/// defines it as the *recording* identifier, and a folder standing for an
/// artist or a release has no recording: carrying the release or artist id
/// under that name would be a different identifier wearing the same label.
/// The `album` and `artist` responses keep it, where it means what it says.
fn directory_child(node: Node) -> Node {
    node.renamed("child")
        .attr("isDir", true)
        .without("musicBrainzId")
}

async fn music_directory(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let id = params.uuid("id")?;
    let overview = overview(state, principal, params).await?;
    let mut directory = Node::new("directory").attr("id", id.to_string());
    if let Some(folder) = overview.folders.iter().find(|item| item.id == id) {
        directory = directory.attr("name", folder.name.clone()).children(
            overview
                .artists
                .iter()
                .filter(|artist| artist.library_id == id)
                .map(|artist| directory_child(artist_node(artist, 0))),
        );
    } else if let Some(artist) = overview.artists.iter().find(|item| item.id == id) {
        directory = directory.attr("name", artist.name.clone()).children(
            overview
                .albums
                .iter()
                .filter(|album| album.artist_id == Some(id))
                .map(|album| directory_child(album_node(album))),
        );
    } else if overview.albums.iter().any(|item| item.id == id) {
        // Only this level needs tracks, and only this album's.
        let detail = state
            .services
            .album(principal.id, id)
            .await
            .map_err(service_protocol)?;
        directory = directory.attr("name", detail.album.title.clone()).children(
            detail
                .songs
                .iter()
                .map(|song| song_node(song).renamed("child")),
        );
    } else {
        return Err(not_found());
    }
    Ok(directory)
}

/// Parameter adapter over [`crate::services::DomainServices::list_albums`].
///
/// The ten ordering modes used to live here, sorted in Rust over a full
/// `catalog_snapshot`. They now resolve in SQL, so this maps Subsonic spelling
/// onto the shared query and does nothing else — which is what M4 asks of a
/// facade, and it stops one album page from reading the tenant's whole
/// catalogue.
async fn album_list(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let order = params
        .first("type")
        .unwrap_or("alphabeticalByName")
        .parse::<AlbumOrder>()
        .map_err(|_| invalid("Invalid album list type"))?;
    let offset = params.usize_or("offset", 0, 100_000)?;
    let size = params.usize_or("size", 10, 500)?;
    // A page of nothing is a valid Subsonic request and used to answer with an
    // empty container. `BrowsePage` rejects a zero limit, so the short-circuit
    // keeps that shape rather than turning it into error code 10.
    if size == 0 {
        return Ok(Node::new("albumList2"));
    }
    let query = AlbumListQuery {
        library_ids: params.uuids("musicFolderId")?,
        order,
        genre: params.first("genre").map(str::to_owned),
        from_year: params.i64_optional("fromYear")?,
        to_year: params.i64_optional("toYear")?,
        page: BrowsePage::new(Some(offset as i64), Some(size as i64)).map_err(service_protocol)?,
    };
    let albums = state
        .services
        .list_albums(principal.id, &query)
        .await
        .map_err(service_protocol)?;
    Ok(Node::new("albumList2").children(albums.iter().map(album_node)))
}

async fn random_songs(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let size = params.usize_or("size", 10, 500)?;
    // A page of nothing is a valid request, as it is for getAlbumList.
    if size == 0 {
        return Ok(Node::new("randomSongs"));
    }
    let songs = state
        .services
        .random_songs(
            principal.id,
            &params.uuids("musicFolderId")?,
            params.first("genre"),
            params.i64_optional("fromYear")?,
            params.i64_optional("toYear")?,
            size as i64,
        )
        .await
        .map_err(service_protocol)?;
    Ok(Node::new("randomSongs").children(songs.iter().map(song_node)))
}

async fn songs_by_genre(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let genre = params.first("genre").ok_or_else(missing)?;
    let offset = params.usize_or("offset", 0, 100_000)?;
    let count = params.usize_or("count", 10, 500)?;
    if count == 0 {
        return Ok(Node::new("songsByGenre"));
    }
    let songs = state
        .services
        .songs_by_genre(
            principal.id,
            &params.uuids("musicFolderId")?,
            genre,
            BrowsePage::new(Some(offset as i64), Some(count as i64)).map_err(service_protocol)?,
        )
        .await
        .map_err(service_protocol)?;
    Ok(Node::new("songsByGenre").children(songs.iter().map(song_node)))
}

async fn search(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let raw_query = params.first("query").ok_or_else(missing)?;
    let artist_count = params.usize_or("artistCount", 20, 500)?;
    let artist_offset = params.usize_or("artistOffset", 0, 100_000)?;
    let album_count = params.usize_or("albumCount", 20, 500)?;
    let album_offset = params.usize_or("albumOffset", 0, 100_000)?;
    let song_count = params.usize_or("songCount", 20, 500)?;
    let song_offset = params.usize_or("songOffset", 0, 100_000)?;

    let folders = params.uuids("musicFolderId")?;

    // Subsonic clients send the literal pair of quotes as the documented
    // match-all query while paging through a complete catalogue. There is
    // nothing to match and FTS5 has no expression meaning "everything", so
    // this is three ordinary listings wearing the search response — paged in
    // SQL rather than sliced out of a full catalogue read, which is what made
    // a client's initial synchronization quadratic in the library.
    if raw_query == "\"\"" {
        let page = |offset: usize, count: usize| {
            BrowsePage::new(Some(offset as i64), Some(count as i64)).map_err(service_protocol)
        };
        let found = state
            .services
            .browse_all(
                principal.id,
                &folders,
                page(artist_offset, artist_count.max(1))?,
                page(album_offset, album_count.max(1))?,
                page(song_offset, song_count.max(1))?,
            )
            .await
            .map_err(service_protocol)?;
        // The service already applied the offsets, so the renderer must not.
        return Ok(search_result(
            found.artists.iter().take(artist_count),
            found.albums.iter().take(album_count),
            found.songs.iter().take(song_count),
            (0, artist_count),
            (0, album_count),
            (0, song_count),
        ));
    }

    let found = state
        .services
        .catalog_search(principal.id, &folders, raw_query)
        .await
        .map_err(internal)?;
    Ok(search_result(
        found.artists.iter(),
        found.albums.iter(),
        found.songs.iter(),
        (artist_offset, artist_count),
        (album_offset, album_count),
        (song_offset, song_count),
    ))
}

/// Renders a `searchResult3` from already-selected entities.
#[allow(clippy::too_many_arguments)]
fn search_result<'a>(
    artists: impl Iterator<Item = &'a ArtistItem>,
    albums: impl Iterator<Item = &'a AlbumItem>,
    songs: impl Iterator<Item = &'a SongItem>,
    (artist_offset, artist_count): (usize, usize),
    (album_offset, album_count): (usize, usize),
    (song_offset, song_count): (usize, usize),
) -> Node {
    Node::new("searchResult3")
        .children(
            artists
                .skip(artist_offset)
                .take(artist_count)
                .map(|artist| artist_node(artist, 0)),
        )
        .children(albums.skip(album_offset).take(album_count).map(album_node))
        .children(songs.skip(song_offset).take(song_count).map(song_node))
}

async fn bookmarks(state: &AppState, principal: &Principal) -> Result<Node, ProtocolError> {
    let bookmarks = state
        .services
        .bookmarks(principal.id)
        .await
        .map_err(internal)?;
    Ok(Node::new("bookmarks").children(
        bookmarks
            .iter()
            .map(|bookmark| bookmark_node(bookmark, &principal.username)),
    ))
}

async fn create_bookmark(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    state
        .services
        .set_bookmark(
            principal.id,
            params.uuid("id")?,
            params.i64("position")?,
            params.first("comment"),
        )
        .await
        .map_err(service_protocol)?;
    Ok(Node::new("createBookmark"))
}

async fn delete_bookmark(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    state
        .services
        .delete_bookmark(principal.id, params.uuid("id")?)
        .await
        .map_err(service_protocol)?;
    Ok(Node::new("deleteBookmark"))
}

async fn playlists(state: &AppState, principal: &Principal) -> Result<Node, ProtocolError> {
    let playlists = state
        .services
        .playlists(principal.id)
        .await
        .map_err(internal)?;
    Ok(Node::new("playlists").children(
        playlists
            .iter()
            .map(|playlist| playlist_node(playlist, &principal.username)),
    ))
}

async fn get_playlist(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let playlist = state
        .services
        .playlist(principal.id, params.uuid("id")?)
        .await
        .map_err(service_protocol)?;
    Ok(playlist_node(&playlist, &principal.username).children(
        playlist
            .songs
            .iter()
            .map(|song| song_node(song).renamed("entry")),
    ))
}

async fn create_playlist(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let ids = params.uuids("songId")?;
    let playlist = if let Some(id) = params.uuid_optional("playlistId")? {
        state
            .services
            // Given a playlistId, songId names every song of the playlist, so
            // the call replaces the track list rather than adding to it. A
            // client that removes a song sends back what remains, and would
            // otherwise see nothing change.
            //
            // The Subsonic contract is frozen: it has no way to ask for a
            // text field to be blanked, so clearing the comment stays off.
            .update_playlist(
                principal.id,
                id,
                None,
                None,
                None,
                &ids,
                &[],
                crate::services::PlaylistClear {
                    comment: false,
                    tracks: true,
                },
            )
            .await
            .map_err(service_protocol)?
    } else {
        state
            .services
            .create_playlist(
                principal.id,
                params.first("name").ok_or_else(missing)?,
                &ids,
            )
            .await
            .map_err(service_protocol)?
    };
    Ok(playlist_node(&playlist, &principal.username).children(
        playlist
            .songs
            .iter()
            .map(|song| song_node(song).renamed("entry")),
    ))
}

async fn update_playlist(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let playlist = state
        .services
        .update_playlist(
            principal.id,
            params.uuid("playlistId")?,
            params.first("name"),
            params.first("comment"),
            params.bool_optional("public")?,
            &params.uuids("songIdToAdd")?,
            &params.usizes("songIndexToRemove")?,
            Default::default(),
        )
        .await
        .map_err(service_protocol)?;
    Ok(playlist_node(&playlist, &principal.username))
}

async fn delete_playlist(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    state
        .services
        .delete_playlist(principal.id, params.uuid("id")?)
        .await
        .map_err(service_protocol)?;
    Ok(Node::new("deletePlaylist"))
}

async fn set_star(
    state: &AppState,
    principal: &Principal,
    params: &Params,
    starred: bool,
) -> Result<Node, ProtocolError> {
    for id in params.uuids("id")? {
        let kind = state
            .services
            .entity_kind(principal.id, id)
            .await
            .map_err(service_protocol)?
            .ok_or_else(not_found)?;
        state
            .services
            .set_star(principal.id, kind, id, starred)
            .await
            .map_err(service_protocol)?;
    }
    for (key, kind) in [("albumId", "album"), ("artistId", "artist")] {
        for id in params.uuids(key)? {
            state
                .services
                .set_star(principal.id, kind, id, starred)
                .await
                .map_err(service_protocol)?;
        }
    }
    Ok(Node::new(if starred { "star" } else { "unstar" }))
}

async fn starred(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    // The three projections already carry `starred_at`, so the nodes emit
    // `starred` themselves. This used to read the whole catalogue and look
    // each starred id up inside it.
    let starred = state
        .services
        .starred(principal.id, &params.uuids("musicFolderId")?)
        .await
        .map_err(service_protocol)?;
    let mut node = Node::new("starred2");
    node.children.extend(
        starred
            .artists
            .iter()
            .map(|summary| artist_node(&summary.artist, summary.album_count as usize)),
    );
    node.children.extend(starred.albums.iter().map(album_node));
    node.children.extend(starred.songs.iter().map(song_node));
    Ok(node)
}

async fn set_rating(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let id = params.uuid("id")?;
    let rating = params.i64("rating")?;
    let kind = state
        .services
        .entity_kind(principal.id, id)
        .await
        .map_err(service_protocol)?
        .ok_or_else(not_found)?;
    state
        .services
        .set_rating(principal.id, kind, id, rating)
        .await
        .map_err(service_protocol)?;
    Ok(Node::new("setRating"))
}

async fn scrobble(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let ids = params.uuids("id")?;
    if ids.is_empty() {
        return Err(missing());
    }
    let times = params
        .all("time")
        .iter()
        .map(|value| value.parse::<i64>().map_err(|_| invalid("Invalid time")))
        .collect::<Result<Vec<_>, _>>()?;
    let submission = params.bool_optional("submission")?.unwrap_or(true);
    for (index, id) in ids.into_iter().enumerate() {
        state
            .services
            .scrobble(principal.id, id, submission, times.get(index).copied())
            .await
            .map_err(service_protocol)?;
    }
    Ok(Node::new("scrobble"))
}

async fn now_playing(state: &AppState, principal: &Principal) -> Result<Node, ProtocolError> {
    let entries = state
        .services
        .now_playing(principal.id)
        .await
        .map_err(internal)?;
    Ok(
        Node::new("nowPlaying").children(entries.iter().map(|(username, song, started)| {
            song_node(song).attr("username", username.clone()).attr(
                "minutesAgo",
                ((chrono::Utc::now().timestamp_millis() - started) / 60_000).max(0),
            )
        })),
    )
}

async fn get_queue(state: &AppState, principal: &Principal) -> Result<Node, ProtocolError> {
    let Some(queue) = state.services.queue(principal.id).await.map_err(internal)? else {
        return Ok(Node::new("playQueue"));
    };
    Ok(Node::new("playQueue")
        .maybe_attr("current", queue.current.map(|id| id.to_string()))
        .attr("position", queue.position_ms)
        .maybe_attr("changedBy", queue.changed_by)
        .attr("changed", iso_time(queue.updated_at))
        .children(queue.songs.iter().map(song_node)))
}

async fn save_queue(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    state
        .services
        .save_queue(
            principal.id,
            &params.uuids("id")?,
            params.uuid_optional("current")?,
            params.i64_or("position", 0)?,
            params.first("c"),
        )
        .await
        .map_err(service_protocol)?;
    Ok(Node::new("savePlayQueue"))
}

async fn media_response(
    state: &AppState,
    principal: &Principal,
    params: &Params,
    download: bool,
    range: Option<&str>,
) -> Result<Response, ProtocolError> {
    let id = params.uuid("id")?;
    let track = state
        .db
        .stream_track_for_user(principal.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(not_found)?;
    let requested_bitrate = params
        .first("maxBitRate")
        .map(|value| value.parse::<u32>())
        .transpose()
        .map_err(|_| invalid("Invalid bitrate"))?
        .filter(|bitrate| *bitrate > 0);
    let format = if download {
        OutputFormat::Raw
    } else if let Some(format) = params.first("format") {
        match format {
            "raw" => OutputFormat::Raw,
            "mp3" => OutputFormat::Mp3,
            "opus" | "ogg" => OutputFormat::Opus,
            _ => return Err(invalid("Unsupported format")),
        }
    } else if requested_bitrate.is_some_and(|limit| {
        track
            .bitrate
            .and_then(|bitrate| u32::try_from(bitrate).ok())
            .is_none_or(|source| source > limit)
    }) {
        // Legacy Subsonic clients such as DSub always send maxBitRate, even
        // when it matches the source. Downsample only when the cap is lower;
        // otherwise preserve direct playback just like Navidrome.
        OutputFormat::Mp3
    } else {
        OutputFormat::Raw
    };
    let query = StreamQuery {
        format,
        bitrate: (format != OutputFormat::Raw)
            .then_some(requested_bitrate)
            .flatten(),
        offset_ms: params
            .first("timeOffset")
            .map(|value| {
                value
                    .parse::<u64>()
                    .ok()
                    .and_then(|seconds| seconds.checked_mul(1000))
                    .ok_or(())
            })
            .transpose()
            .map_err(|_| invalid("Invalid time offset"))?
            .unwrap_or(0),
    };
    match state.media.serve(principal.id, track, query, range).await {
        Ok(mut response) => {
            if download {
                response.headers_mut().insert(
                    header::CONTENT_DISPOSITION,
                    "attachment".parse().expect("static header value"),
                );
            }
            Ok(response)
        }
        Err(MediaError::NotFound | MediaError::Unauthorized) => Err(not_found()),
        Err(MediaError::InvalidRequest) => Err(invalid("Invalid media parameters")),
        Err(error @ (MediaError::RangeNotSatisfiable(_) | MediaError::Busy)) => {
            Ok(error.into_response())
        }
        Err(MediaError::Internal) => Err(internal("media service failed")),
    }
}

async fn cover_art_response(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Response, ProtocolError> {
    let id = params.first("id").ok_or_else(missing)?;
    let (hash, format) = state
        .services
        .artwork_for_user(principal.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(not_found)?;
    let (mime, bytes) = crate::media::read_artwork(&state.artwork_dir, &hash, &format)
        .await
        .ok_or_else(not_found)?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "private, max-age=86400"),
        ],
        bytes,
    )
        .into_response())
}

async fn shares(
    state: &AppState,
    principal: &Principal,
    method: &str,
    params: &Params,
) -> Result<Node, ProtocolError> {
    match method {
        "getShares" => Ok(Node::new("shares").children(
            state
                .services
                .shares(principal.id)
                .await
                .map_err(internal)?
                .iter()
                .map(|share| share_node(share, &principal.username, state.public_url.as_deref())),
        )),
        "createShare" => {
            let share = state
                .services
                .create_share(
                    principal.id,
                    &params.uuids("id")?,
                    params.first("description"),
                    params.first("expires").map(parse_time).transpose()?,
                )
                .await
                .map_err(service_protocol)?;
            Ok(Node::new("shares").child(share_node(
                &share,
                &principal.username,
                state.public_url.as_deref(),
            )))
        }
        "updateShare" => {
            let share = state
                .services
                .update_share(
                    principal.id,
                    params.uuid("id")?,
                    params.first("description"),
                    params.first("expires").map(parse_time).transpose()?,
                    Default::default(),
                )
                .await
                .map_err(service_protocol)?;
            Ok(Node::new("shares").child(share_node(
                &share,
                &principal.username,
                state.public_url.as_deref(),
            )))
        }
        "deleteShare" => {
            state
                .services
                .delete_share(principal.id, params.uuid("id")?)
                .await
                .map_err(service_protocol)?;
            Ok(Node::new("deleteShare"))
        }
        _ => unreachable!("share method dispatch is exhaustive"),
    }
}

async fn admin(
    state: &AppState,
    principal: &Principal,
    method: &str,
    params: &Params,
) -> Result<Node, ProtocolError> {
    if principal.role != AccountRole::Admin {
        return Err(ProtocolError {
            code: 50,
            message: "User is not authorized for the given operation",
        });
    }
    match method {
        "getUser" => {
            let username = params.first("username").unwrap_or(&principal.username);
            let user = state
                .services
                .users(principal.id)
                .await
                .map_err(service_protocol)?
                .into_iter()
                .find(|user| user.username.eq_ignore_ascii_case(username))
                .ok_or_else(not_found)?;
            Ok(user_node(&user))
        }
        "getUsers" => Ok(Node::new("users").children(
            state
                .services
                .users(principal.id)
                .await
                .map_err(service_protocol)?
                .iter()
                .map(user_node),
        )),
        "createUser" => {
            let password =
                decode_credential_password(params.first("password").ok_or_else(missing)?)?;
            let folders = params.uuids("musicFolderId")?;
            let folders = params.first("musicFolderId").is_some().then_some(folders);
            let user = state
                .services
                .create_subsonic_user(
                    principal.id,
                    params.first("username").ok_or_else(missing)?,
                    &password,
                    params.bool_optional("adminRole")?.unwrap_or(false),
                    folders.as_deref(),
                )
                .await
                .map_err(service_protocol)?;
            Ok(user_node(&user))
        }
        "updateUser" => {
            let folders = params.uuids("musicFolderId")?;
            let folders = params.first("musicFolderId").is_some().then_some(folders);
            let password = params
                .first("password")
                .map(decode_credential_password)
                .transpose()?;
            let user = state
                .services
                .update_user(
                    principal.id,
                    params.first("username").ok_or_else(missing)?,
                    crate::services::UserUpdate {
                        admin: params.bool_optional("adminRole")?,
                        disabled: params.bool_optional("locked")?,
                        folder_ids: folders.as_deref(),
                        subsonic_password: password.as_deref(),
                        web_password: None,
                    },
                )
                .await
                .map_err(service_protocol)?;
            Ok(user_node(&user))
        }
        "deleteUser" => {
            state
                .services
                .delete_user(principal.id, params.first("username").ok_or_else(missing)?)
                .await
                .map_err(service_protocol)?;
            Ok(Node::new("deleteUser"))
        }
        "changePassword" => {
            let password =
                decode_credential_password(params.first("password").ok_or_else(missing)?)?;
            state
                .services
                .change_subsonic_password(
                    principal.id,
                    params.first("username").ok_or_else(missing)?,
                    &password,
                )
                .await
                .map_err(service_protocol)?;
            Ok(Node::new("changePassword"))
        }
        _ => unreachable!("admin method dispatch is exhaustive"),
    }
}

fn artist_node(artist: &ArtistItem, album_count: usize) -> Node {
    Node::new("artist")
        .attr("id", artist.id.to_string())
        .attr("name", artist.name.clone())
        .attr("albumCount", album_count as i64)
        .maybe_attr("coverArt", artist.artwork_hash.clone())
        .maybe_attr("starred", artist.starred_at.map(iso_time))
        .maybe_attr("userRating", artist.user_rating)
        // An OpenSubsonic addition, under the presence rule the media items
        // already follow: on an artist the identifier means the artist, and it
        // is emitted empty rather than omitted so a client can tell an untagged
        // artist from a server that does not read the tag at all.
        .attr(
            "musicBrainzId",
            artist.musicbrainz_id.clone().unwrap_or_default(),
        )
}

/// `songCount` and `duration` come from the album projection rather than from
/// a slice of loaded tracks: counting them caller-side is what forced every
/// album listing to materialise the tenant's whole track list first.
fn album_node(album: &AlbumItem) -> Node {
    Node::new("album")
        .attr("id", album.id.to_string())
        .attr("name", album.title.clone())
        .attr("title", album.title.clone())
        .maybe_attr("artist", album.artist.clone())
        .maybe_attr("artistId", album.artist_id.map(|id| id.to_string()))
        .maybe_attr("coverArt", album.artwork_hash.clone())
        .maybe_attr("year", album.year)
        .maybe_attr("starred", album.starred_at.map(iso_time))
        .maybe_attr("userRating", album.user_rating)
        .attr("songCount", album.song_count)
        .attr("duration", album.duration_ms / 1000)
        .attr("created", iso_time(album.created_at))
        // OpenSubsonic additions, under the same presence rule as `song`.
        .attr("isCompilation", album.is_compilation)
        .attr("playCount", album.play_count)
        .attr("displayArtist", album.artist.clone().unwrap_or_default())
        .maybe_attr("played", album.last_played_at.map(iso_time))
        .children(album.artists.iter().map(|artist| {
            Node::new("artists")
                .attr("id", artist.id.to_string())
                .attr("name", artist.name.clone())
        }))
        .children(
            album
                .genres
                .iter()
                .map(|genre| Node::new("genres").attr("name", genre.clone())),
        )
        // On an album the identifier means the release, not the recording the
        // song carries. It is derived from the album's own tracks at scan time,
        // so it is a plain column read here.
        .attr(
            "musicBrainzId",
            album.musicbrainz_id.clone().unwrap_or_default(),
        )
}

fn song_node(song: &SongItem) -> Node {
    Node::new("song")
        .attr("id", song.id.to_string())
        .attr(
            "parent",
            song.album_id.unwrap_or(song.library_id).to_string(),
        )
        .attr("isDir", false)
        .attr("title", song.title.clone())
        .maybe_attr("album", song.album.clone())
        .maybe_attr("artist", song.artist.clone())
        .maybe_attr("genre", song.genre.clone())
        .maybe_attr("year", song.year)
        .maybe_attr("track", song.track)
        .maybe_attr("discNumber", song.disc)
        .attr("duration", song.duration_ms / 1000)
        .maybe_attr("bitRate", song.bitrate)
        .attr("size", song.size)
        .attr("suffix", song.suffix.clone())
        .attr("contentType", content_type(&song.suffix))
        .attr("type", "music")
        .maybe_attr("coverArt", song.artwork_hash.clone())
        .maybe_attr("albumId", song.album_id.map(|id| id.to_string()))
        .maybe_attr("artistId", song.artist_id.map(|id| id.to_string()))
        .maybe_attr("starred", song.starred_at.map(iso_time))
        .maybe_attr("userRating", song.user_rating)
        .attr("created", iso_time(song.created_at))
        // From here down the fields are OpenSubsonic additions, and they follow
        // its presence rule rather than the omission rule the frozen 1.16
        // fields above use: a field the server supports is emitted even when
        // the value is unknown, because presence is the only way a client can
        // tell "this server does not implement it" from "this track has none".
        .attr("mediaType", "song")
        .attr("isVideo", false)
        .attr("samplingRate", song.sample_rate.unwrap_or_default())
        .attr("channelCount", song.channels.unwrap_or_default())
        .attr("bitDepth", song.bit_depth.unwrap_or_default())
        .attr("playCount", song.play_count)
        .attr("displayArtist", song.artist.clone().unwrap_or_default())
        // `played` is the one exception. Its default would be the empty
        // string, which is not a timestamp: a client parsing it strictly would
        // fail on every track nobody has played. `playCount` is always present
        // and already tells the client play statistics are supported.
        .maybe_attr("played", song.last_played_at.map(iso_time))
        .children(song.artists.iter().map(|artist| {
            Node::new("artists")
                .attr("id", artist.id.to_string())
                .attr("name", artist.name.clone())
        }))
        .children(
            song.genres
                .iter()
                .map(|genre| Node::new("genres").attr("name", genre.clone())),
        )
        .children(song.album_artist_id.iter().map(|id| {
            Node::new("albumArtists")
                .attr("id", id.to_string())
                .attr("name", song.album_artist.clone().unwrap_or_default())
        }))
        .attr(
            "displayAlbumArtist",
            song.album_artist.clone().unwrap_or_default(),
        )
        .attr(
            "musicBrainzId",
            song.musicbrainz_id.clone().unwrap_or_default(),
        )
        .attr("bpm", song.bpm.unwrap_or_default())
        .attr("sortName", song.sort_name.clone().unwrap_or_default())
        .attr("comment", song.comment.clone().unwrap_or_default())
        .children(
            song.isrc
                .iter()
                .map(|isrc| Node::new("isrc").text(isrc.clone())),
        )
        .children(
            song.moods
                .iter()
                .map(|mood| Node::new("moods").text(mood.clone())),
        )
        .attr(
            "explicitStatus",
            song.explicit_status.clone().unwrap_or_default(),
        )
        // ReplayGain is the one addition whose *members* are omitted when
        // unknown, on the specification's own instruction. The container is
        // still always present, because that is what says the server reads
        // gain tags at all; an untagged track carries an empty one.
        .child(
            Node::new("replayGain")
                .maybe_attr("trackGain", song.replay_gain_track_gain)
                .maybe_attr("trackPeak", song.replay_gain_track_peak)
                .maybe_attr("albumGain", song.replay_gain_album_gain)
                .maybe_attr("albumPeak", song.replay_gain_album_peak),
        )
}

/// `owner` is the caller: playlist reads are already scoped to their owner, so
/// there is no other name this could carry. Leaving it empty made Feishin
/// treat every playlist as someone else's and refuse to edit it.
/// `bookmarkPosition` is set on the entry rather than on every song node: it
/// is a legacy optional field, and a track only has a position inside the
/// bookmark that holds it.
fn bookmark_node(bookmark: &crate::services::BookmarkItem, owner: &str) -> Node {
    Node::new("bookmark")
        .attr("position", bookmark.position_ms)
        .attr("username", owner)
        .maybe_attr("comment", bookmark.comment.clone())
        .attr("created", iso_time(bookmark.created_at))
        .attr("changed", iso_time(bookmark.updated_at))
        .child(
            song_node(&bookmark.song)
                .renamed("entry")
                .attr("bookmarkPosition", bookmark.position_ms),
        )
}

fn playlist_node(playlist: &PlaylistItem, owner: &str) -> Node {
    Node::new("playlist")
        .attr("id", playlist.id.to_string())
        .attr("name", playlist.name.clone())
        .maybe_attr("comment", playlist.comment.clone())
        .attr("owner", owner)
        .attr("public", playlist.public)
        .attr("songCount", playlist.songs.len() as i64)
        .attr(
            "duration",
            playlist
                .songs
                .iter()
                .map(|song| song.duration_ms / 1000)
                .sum::<i64>(),
        )
        .attr("created", iso_time(playlist.created_at))
        .attr("changed", iso_time(playlist.updated_at))
}

fn user_node(user: &crate::services::UserItem) -> Node {
    Node::new("user")
        .attr("username", user.username.clone())
        .attr("scrobblingEnabled", true)
        .attr("adminRole", user.role == AccountRole::Admin)
        .attr("settingsRole", user.role == AccountRole::Admin)
        .attr("downloadRole", true)
        .attr("uploadRole", false)
        .attr("playlistRole", true)
        .attr("coverArtRole", true)
        .attr("commentRole", false)
        .attr("podcastRole", false)
        .attr("streamRole", true)
        .attr("jukeboxRole", false)
        .attr("shareRole", true)
        .attr("videoConversionRole", false)
        .children(
            user.folder_ids
                .iter()
                .map(|id| Node::new("folder").text(id.to_string())),
        )
}

fn share_node(share: &crate::services::ShareItem, owner: &str, public_url: Option<&str>) -> Node {
    let url = share.url_token.as_ref().map(|token| {
        let path = format!("/share/{token}");
        external_url(public_url, &path)
    });
    Node::new("share")
        .attr("id", share.id.to_string())
        .maybe_attr("url", url)
        .maybe_attr("description", share.description.clone())
        .maybe_attr("expires", share.expires_at.map(iso_time))
        .attr("username", owner)
        .attr("created", iso_time(share.created_at))
        .attr("visitCount", share.visit_count)
        .children(
            share
                .songs
                .iter()
                .map(|song| song_node(song).renamed("entry")),
        )
}

fn external_url(base: Option<&str>, path: &str) -> String {
    base.map_or_else(|| path.to_owned(), |base| format!("{base}{path}"))
}

fn ok_node() -> Node {
    Node::new("subsonic-response")
        .attr("xmlns", XMLNS)
        .attr("status", "ok")
        .attr("version", SUBSONIC_VERSION)
        .attr("type", "waveflow")
        .attr("serverVersion", env!("CARGO_PKG_VERSION"))
        .attr("openSubsonic", true)
}

fn error_node(code: i64, message: &'static str) -> Node {
    Node::new("subsonic-response")
        .attr("xmlns", XMLNS)
        .attr("status", "failed")
        .attr("version", SUBSONIC_VERSION)
        .attr("type", "waveflow")
        .attr("serverVersion", env!("CARGO_PKG_VERSION"))
        .attr("openSubsonic", true)
        .child(
            Node::new("error")
                .attr("code", code)
                .attr("message", message),
        )
}

fn render_protocol(node: Node, json: bool) -> Response {
    if json {
        let mut root = Map::new();
        root.insert(node.name.clone(), node_json(&node));
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            Value::Object(root).to_string(),
        )
            .into_response()
    } else {
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
            node_xml(&node),
        )
            .into_response()
    }
}

fn node_json(node: &Node) -> Value {
    // An array-typed element is its children, not an object wrapping them —
    // whether it holds none or several. Applying this only when empty would
    // hand a strictly typed client `[]` on an empty catalogue and an object on
    // a populated one, which is worse than being wrong consistently.
    if json_array_node(&node.name) {
        return Value::Array(node.children.iter().map(node_json).collect());
    }
    if node.attrs.is_empty() && node.children.is_empty() {
        if let Some(text) = &node.text {
            return Value::String(text.clone());
        }
    }
    let mut map = Map::new();
    for (key, value) in &node.attrs {
        if key != "xmlns" {
            map.insert(key.clone(), value.clone());
        }
    }
    let mut grouped: BTreeMap<&str, Vec<&Node>> = BTreeMap::new();
    for child in &node.children {
        grouped.entry(&child.name).or_default().push(child);
    }
    for (name, children) in grouped {
        let value = if children.len() == 1 && !json_array_field(&node.name, name) {
            node_json(children[0])
        } else {
            Value::Array(children.into_iter().map(node_json).collect())
        };
        map.insert(name.to_owned(), value);
    }
    for name in json_required_array_fields(&node.name) {
        map.entry((*name).to_owned())
            .or_insert_with(|| Value::Array(Vec::new()));
    }
    if let Some(text) = &node.text {
        map.insert("value".into(), Value::String(text.clone()));
    }
    Value::Object(map)
}

/// Elements the OpenSubsonic specification types as a JSON array rather than an
/// object. They must serialise as `[]` when empty; an empty object breaks
/// strictly typed clients that decode the field into a list.
fn json_array_node(name: &str) -> bool {
    matches!(name, "openSubsonicExtensions")
}

fn json_required_array_fields(parent: &str) -> &'static [&'static str] {
    match parent {
        "lyricsList" => &["structuredLyrics"],
        "structuredLyrics" => &["line"],
        // Emitted as `[]` rather than omitted when a track has no credited
        // artist or no genre: under the OpenSubsonic presence rule an absent
        // key means the server does not support the field at all.
        "song" | "entry" | "child" => &["artists", "genres", "isrc", "moods", "albumArtists"],
        "album" => &["artists", "genres"],
        _ => &[],
    }
}

/// Extensions this server actually implements, with their supported versions.
///
/// The list was empty, which told every third-party client that WaveFlow
/// supports nothing optional — so a client that could have posted a long
/// request, authenticated with an API key or seeked a transcode fell back to
/// the lowest common denominator instead.
///
/// **Only advertise what is implemented and covered by tests.** Announcing an
/// extension the server does not honour is worse than announcing none: the
/// client stops probing and starts relying on it.
///
/// The specification defines no XML shape for this method, so `versions`
/// renders as a JSON array here and stringifies as `"[1]"` in the XML branch.
/// Clients that use the method request JSON.
fn open_subsonic_extensions() -> Node {
    let extension = |name: &str, versions: Vec<i64>| {
        Node::new("openSubsonicExtension").attr("name", name).attr(
            "versions",
            Value::Array(versions.into_iter().map(Value::from).collect()),
        )
    };
    Node::new("openSubsonicExtensions")
        // POST with application/x-www-form-urlencoded, for requests too long
        // for a query string.
        .child(extension("formPost", vec![1]))
        // `apiKey` in place of the u/p and u/t/s pairs.
        .child(extension("apiKeyAuthentication", vec![1]))
        // `timeOffset` on stream, honoured for transcoded output.
        .child(extension("transcodeOffset", vec![1]))
        // Structured plain or line-synchronised lyrics by stable song UUID.
        .child(extension("songLyrics", vec![1]))
}

fn json_array_field(parent: &str, name: &str) -> bool {
    matches!(
        (parent, name),
        ("musicFolders", "musicFolder")
            | ("indexes", "index")
            | ("artists", "index")
            | ("index", "artist")
            | ("artist", "album")
            | ("album", "song")
            | ("genres", "genre")
            | ("directory", "child")
            | ("albumList", "album")
            | ("albumList2", "album")
            | ("randomSongs", "song")
            | ("songsByGenre", "song")
            | ("searchResult3" | "searchResult2", "artist" | "album" | "song")
            | ("playlists", "playlist")
            | ("playlist", "entry")
            | ("bookmarks", "bookmark")
            | ("starred2" | "starred", "artist" | "album" | "song")
            | ("nowPlaying", "song")
            | ("playQueue", "song")
            | ("shares", "share")
            | ("share", "entry")
            | ("users", "user")
            | ("user", "folder")
            | ("openSubsonicExtensions", "openSubsonicExtension")
            | ("lyricsList", "structuredLyrics")
            | ("structuredLyrics", "line")
            // A media item is rendered as `song`, and renamed to `entry` inside
            // a playlist or share and to `child` inside a directory. Its
            // OpenSubsonic relations are arrays under all three names.
            | ("song" | "entry" | "child" | "album", "artists" | "genres")
            | ("song" | "entry" | "child", "isrc" | "moods" | "albumArtists")
    )
}

fn empty_success_method(method: &str) -> bool {
    matches!(
        method,
        "updatePlaylist"
            | "deletePlaylist"
            | "star"
            | "unstar"
            | "setRating"
            | "scrobble"
            | "savePlayQueue"
            | "createBookmark"
            | "deleteBookmark"
            | "deleteShare"
            | "createUser"
            | "updateUser"
            | "deleteUser"
            | "changePassword"
    )
}

fn decode_credential_password(value: &str) -> Result<String, ProtocolError> {
    let bytes = match value.strip_prefix("enc:") {
        Some(encoded) => hex::decode(encoded).map_err(|_| invalid("Invalid password encoding"))?,
        None => value.as_bytes().to_vec(),
    };
    String::from_utf8(bytes).map_err(|_| invalid("Invalid password encoding"))
}

fn node_xml(node: &Node) -> String {
    let mut output = String::new();
    write_xml(node, &mut output);
    output
}

fn write_xml(node: &Node, output: &mut String) {
    output.push('<');
    output.push_str(&node.name);
    for (key, value) in &node.attrs {
        output.push(' ');
        output.push_str(key);
        output.push_str("=\"");
        output.push_str(&xml_escape(&value_string(value)));
        output.push('"');
    }
    if node.children.is_empty() && node.text.is_none() {
        output.push_str("/>");
        return;
    }
    output.push('>');
    if let Some(text) = &node.text {
        output.push_str(&xml_escape(text));
    }
    for child in &node.children {
        write_xml(child, output);
    }
    output.push_str("</");
    output.push_str(&node.name);
    output.push('>');
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

fn parse_pairs(raw: &str) -> Result<Params, ProtocolError> {
    serde_urlencoded::from_str::<Vec<(String, String)>>(raw)
        .map(Params)
        .map_err(|_| invalid("Invalid parameters"))
}

fn content_type(suffix: &str) -> &'static str {
    match suffix {
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "ogg" | "opus" => "audio/ogg",
        "m4a" | "mp4" => "audio/mp4",
        "aac" => "audio/aac",
        "dsf" | "dff" => "audio/dsd",
        _ => "application/octet-stream",
    }
}
fn iso_time(millis: i64) -> String {
    chrono::DateTime::from_timestamp_millis(millis)
        .unwrap_or_default()
        .to_rfc3339()
}

fn parse_time(value: &str) -> Result<i64, ProtocolError> {
    if let Ok(millis) = value.parse::<i64>() {
        return Ok(millis);
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|value| value.timestamp_millis())
        .map_err(|_| invalid("Invalid date"))
}
fn value_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        _ => value.to_string(),
    }
}
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn auth_error() -> ProtocolError {
    ProtocolError {
        code: 40,
        message: "Wrong username or password",
    }
}
fn missing() -> ProtocolError {
    ProtocolError {
        code: 10,
        message: "Required parameter is missing",
    }
}
fn invalid(message: &'static str) -> ProtocolError {
    ProtocolError { code: 10, message }
}
fn not_found() -> ProtocolError {
    ProtocolError {
        code: 70,
        message: "The requested data was not found",
    }
}
fn internal(error: impl std::fmt::Display) -> ProtocolError {
    tracing::error!(error = %error, "Subsonic service failure");
    ProtocolError {
        code: 0,
        message: "Internal server error",
    }
}
fn service_protocol(error: ServiceError) -> ProtocolError {
    match error {
        ServiceError::NotFound => not_found(),
        ServiceError::Forbidden => ProtocolError {
            code: 50,
            message: "User is not authorized for the given operation",
        },
        ServiceError::Invalid => invalid("Invalid parameters"),
        ServiceError::Conflict => ProtocolError {
            code: 0,
            message: "Conflict",
        },
        other => internal(other),
    }
}
