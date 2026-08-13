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
use rand::seq::SliceRandom;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::{
    database::AccountRole,
    media::{MediaError, OutputFormat, StreamQuery},
    security,
    services::{AlbumItem, ArtistItem, CatalogSnapshot, PlaylistItem, ServiceError, SongItem},
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

    fn children(mut self, children: impl IntoIterator<Item = Node>) -> Self {
        self.children.extend(children);
        self
    }
}

#[derive(Debug)]
struct ProtocolError {
    code: i64,
    message: &'static str,
    status: StatusCode,
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
    let format_hint = request.uri().query().unwrap_or_default().to_owned();
    match handle_inner(state, raw_method, request).await {
        Ok(response) => response,
        Err(error) => render_protocol(
            error_node(error.code, error.message),
            wants_json_query(&format_hint),
            error.status,
        ),
    }
}

async fn handle_inner(
    state: AppState,
    raw_method: String,
    request: Request,
) -> Result<Response, ProtocolError> {
    let method = raw_method.strip_suffix(".view").unwrap_or(&raw_method);
    let request_method = request.method().clone();
    let query = request.uri().query().unwrap_or_default().to_owned();
    let range = request
        .headers()
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let mut params = parse_pairs(&query)?;
    if request_method == Method::POST {
        let content_type = request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if !content_type.starts_with("application/x-www-form-urlencoded") {
            return Err(invalid("POST requires application/x-www-form-urlencoded"));
        }
        let body = to_bytes(request.into_body(), MAX_FORM_BYTES)
            .await
            .map_err(|_| invalid("Invalid form body"))?;
        params.0.extend(
            parse_pairs(std::str::from_utf8(&body).map_err(|_| invalid("Invalid form body"))?)?.0,
        );
    }
    let wants_json = params.first("f").is_some_and(|value| value == "json");
    if is_symfonium_discovery_probe(method, &request_method, &params) {
        return Ok(render_protocol(ok_node(), wants_json, StatusCode::OK));
    }
    let principal = authenticate(&state, &params).await?;

    if matches!(method, "stream" | "download") {
        return media_response(
            &state,
            &principal,
            &params,
            method == "download",
            range.as_deref(),
        )
        .await;
    }
    if method == "getCoverArt" {
        return cover_art_response(&state, &principal, &params).await;
    }

    let payload = dispatch(&state, &principal, method, &params).await?;
    let root = if method == "ping" || empty_success_method(method) {
        ok_node()
    } else {
        ok_node().child(payload)
    };
    Ok(render_protocol(root, wants_json, StatusCode::OK))
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
        // Symfonium includes bookmarks in its initial sync even when the
        // server does not expose audiobook progress. Returning the standard
        // empty container keeps that optional capability non-destructive.
        "getBookmarks" => Ok(Node::new("bookmarks")),
        "getMusicFolders" => {
            let snapshot = state
                .services
                .catalog_snapshot(principal.id, &[])
                .await
                .map_err(internal)?;
            Ok(
                Node::new("musicFolders").children(snapshot.folders.into_iter().map(|folder| {
                    Node::new("musicFolder")
                        .attr("id", folder.id.to_string())
                        .attr("name", folder.name)
                })),
            )
        }
        "getIndexes" => indexes(state, principal, params, false).await,
        "getArtists" => indexes(state, principal, params, true).await,
        "getArtist" => get_artist(state, principal, params).await,
        "getAlbum" => get_album(state, principal, params).await,
        "getSong" => get_song(state, principal, params).await,
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
        "getPlaylists" => playlists(state, principal).await,
        "getPlaylist" => get_playlist(state, principal, params).await,
        "createPlaylist" => create_playlist(state, principal, params).await,
        "updatePlaylist" => update_playlist(state, principal, params).await,
        "deletePlaylist" => delete_playlist(state, principal, params).await,
        "star" => set_star(state, principal, params, true).await,
        "unstar" => set_star(state, principal, params, false).await,
        "getStarred2" => starred(state, principal, params).await,
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
            status: StatusCode::NOT_FOUND,
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
            status: StatusCode::TOO_MANY_REQUESTS,
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

async fn snapshot(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<CatalogSnapshot, ProtocolError> {
    let folders = params.uuids("musicFolderId")?;
    state
        .services
        .catalog_snapshot(principal.id, &folders)
        .await
        .map_err(internal)
}

async fn indexes(
    state: &AppState,
    principal: &Principal,
    params: &Params,
    id3: bool,
) -> Result<Node, ProtocolError> {
    let snapshot = snapshot(state, principal, params).await?;
    let mut groups: BTreeMap<char, Vec<ArtistItem>> = BTreeMap::new();
    for artist in snapshot.artists {
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
                        snapshot
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
    let id = params.uuid("id")?;
    let snapshot = snapshot(state, principal, params).await?;
    let artist = snapshot
        .artists
        .iter()
        .find(|artist| artist.id == id)
        .ok_or_else(not_found)?;
    let albums = snapshot
        .albums
        .iter()
        .filter(|album| album.artist_id == Some(id))
        .map(|album| album_node(album, &snapshot.songs))
        .collect::<Vec<_>>();
    Ok(artist_node(artist, albums.len()).children(albums))
}

async fn get_album(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let id = params.uuid("id")?;
    let snapshot = snapshot(state, principal, params).await?;
    let album = snapshot
        .albums
        .iter()
        .find(|album| album.id == id)
        .ok_or_else(not_found)?;
    Ok(album_node(album, &snapshot.songs).children(
        snapshot
            .songs
            .iter()
            .filter(|song| song.album_id == Some(id))
            .map(song_node),
    ))
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

async fn genres(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let snapshot = snapshot(state, principal, params).await?;
    let mut counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for song in &snapshot.songs {
        for genre in split_multi(song.genre.as_deref()) {
            counts.entry(genre).or_default().0 += 1;
        }
    }
    for album in &snapshot.albums {
        let mut seen = Vec::new();
        for song in snapshot
            .songs
            .iter()
            .filter(|song| song.album_id == Some(album.id))
        {
            for genre in split_multi(song.genre.as_deref()) {
                if !seen.contains(&genre) {
                    seen.push(genre.clone());
                    counts.entry(genre).or_default().1 += 1;
                }
            }
        }
    }
    Ok(
        Node::new("genres").children(counts.into_iter().map(|(name, (songs, albums))| {
            Node::new("genre")
                .attr("songCount", songs as i64)
                .attr("albumCount", albums as i64)
                .text(name)
        })),
    )
}

async fn music_directory(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let id = params.uuid("id")?;
    let snapshot = snapshot(state, principal, params).await?;
    let mut directory = Node::new("directory").attr("id", id.to_string());
    if let Some(folder) = snapshot.folders.iter().find(|item| item.id == id) {
        directory = directory.attr("name", folder.name.clone()).children(
            snapshot
                .artists
                .iter()
                .filter(|artist| artist.library_id == id)
                .map(|artist| artist_node(artist, 0).renamed("child").attr("isDir", true)),
        );
    } else if let Some(artist) = snapshot.artists.iter().find(|item| item.id == id) {
        directory = directory.attr("name", artist.name.clone()).children(
            snapshot
                .albums
                .iter()
                .filter(|album| album.artist_id == Some(id))
                .map(|album| {
                    album_node(album, &snapshot.songs)
                        .renamed("child")
                        .attr("isDir", true)
                }),
        );
    } else if let Some(album) = snapshot.albums.iter().find(|item| item.id == id) {
        directory = directory.attr("name", album.title.clone()).children(
            snapshot
                .songs
                .iter()
                .filter(|song| song.album_id == Some(id))
                .map(|song| song_node(song).renamed("child")),
        );
    } else {
        return Err(not_found());
    }
    Ok(directory)
}

async fn album_list(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let kind = params.first("type").unwrap_or("alphabeticalByName");
    let offset = params.usize_or("offset", 0, 100_000)?;
    let size = params.usize_or("size", 10, 500)?;
    let snapshot = snapshot(state, principal, params).await?;
    let mut albums = snapshot.albums.iter().collect::<Vec<_>>();
    match kind {
        "alphabeticalByName" => albums.sort_by_key(|album| (album.title.to_lowercase(), album.id)),
        "alphabeticalByArtist" => albums.sort_by_key(|album| {
            (
                album.artist.clone().unwrap_or_default().to_lowercase(),
                album.title.to_lowercase(),
                album.id,
            )
        }),
        "newest" => albums.sort_by_key(|album| (std::cmp::Reverse(album.created_at), album.id)),
        "highest" => {
            albums.retain(|album| album.user_rating.is_some_and(|rating| rating > 0));
            albums.sort_by_key(|album| {
                (
                    std::cmp::Reverse(album.user_rating.unwrap_or_default()),
                    album.title.to_lowercase(),
                    album.id,
                )
            });
        }
        "frequent" => {
            albums.retain(|album| album.play_count > 0);
            albums.sort_by_key(|album| {
                (
                    std::cmp::Reverse(album.play_count),
                    album.title.to_lowercase(),
                    album.id,
                )
            });
        }
        "recent" => {
            albums.retain(|album| album.last_played_at.is_some());
            albums.sort_by_key(|album| {
                (
                    std::cmp::Reverse(album.last_played_at.unwrap_or_default()),
                    album.title.to_lowercase(),
                    album.id,
                )
            });
        }
        "starred" => {
            albums.retain(|album| album.starred_at.is_some());
            albums.sort_by_key(|album| {
                (
                    std::cmp::Reverse(album.starred_at.unwrap_or_default()),
                    album.title.to_lowercase(),
                    album.id,
                )
            });
        }
        "random" => albums.shuffle(&mut rand::thread_rng()),
        "byYear" => {
            let from = params.i64_or("fromYear", i64::MIN)?;
            let to = params.i64_or("toYear", i64::MAX)?;
            albums.retain(|album| {
                album
                    .year
                    .is_some_and(|year| year >= from.min(to) && year <= from.max(to))
            });
            if from <= to {
                albums.sort_by_key(|album| (album.year, album.title.to_lowercase(), album.id));
            } else {
                albums.sort_by_key(|album| {
                    (
                        std::cmp::Reverse(album.year),
                        album.title.to_lowercase(),
                        album.id,
                    )
                });
            }
        }
        "byGenre" => {
            let genre = params.first("genre").ok_or_else(missing)?;
            albums.retain(|album| {
                snapshot
                    .songs
                    .iter()
                    .filter(|song| song.album_id == Some(album.id))
                    .flat_map(|song| split_multi(song.genre.as_deref()))
                    .any(|value| value.eq_ignore_ascii_case(genre))
            });
            albums.sort_by_key(|album| (album.title.to_lowercase(), album.id));
        }
        _ => return Err(invalid("Invalid album list type")),
    }
    Ok(Node::new("albumList2").children(
        albums
            .into_iter()
            .skip(offset)
            .take(size)
            .map(|album| album_node(album, &snapshot.songs)),
    ))
}

async fn random_songs(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let size = params.usize_or("size", 10, 500)?;
    let mut songs = snapshot(state, principal, params).await?.songs;
    if let Some(genre) = params.first("genre") {
        songs.retain(|song| {
            split_multi(song.genre.as_deref())
                .iter()
                .any(|value| value.eq_ignore_ascii_case(genre))
        });
    }
    let from = params.i64_or("fromYear", i64::MIN)?;
    let to = params.i64_or("toYear", i64::MAX)?;
    if params.first("fromYear").is_some() || params.first("toYear").is_some() {
        songs.retain(|song| {
            song.year
                .is_some_and(|year| year >= from.min(to) && year <= from.max(to))
        });
    }
    songs.shuffle(&mut rand::thread_rng());
    Ok(Node::new("randomSongs").children(songs.iter().take(size).map(song_node)))
}

async fn songs_by_genre(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let genre = params.first("genre").ok_or_else(missing)?;
    let offset = params.usize_or("offset", 0, 100_000)?;
    let count = params.usize_or("count", 10, 500)?;
    let songs = snapshot(state, principal, params).await?.songs;
    Ok(Node::new("songsByGenre").children(
        songs
            .iter()
            .filter(|song| {
                split_multi(song.genre.as_deref())
                    .iter()
                    .any(|value| value.eq_ignore_ascii_case(genre))
            })
            .skip(offset)
            .take(count)
            .map(song_node),
    ))
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

    // Subsonic clients use the literal pair of quotes as the documented
    // match-all query while paginating a complete catalogue. That path stays on
    // the full snapshot: there is nothing to match, and FTS5 has no expression
    // meaning "everything".
    if raw_query == "\"\"" {
        let snapshot = snapshot(state, principal, params).await?;
        return Ok(search_result(
            snapshot.artists.iter(),
            snapshot.albums.iter(),
            snapshot.songs.iter(),
            &snapshot.songs,
            (artist_offset, artist_count),
            (album_offset, album_count),
            (song_offset, song_count),
        ));
    }

    let folders = params.uuids("musicFolderId")?;
    let found = state
        .services
        .catalog_search(principal.id, &folders, raw_query)
        .await
        .map_err(internal)?;
    Ok(search_result(
        found.artists.iter(),
        found.albums.iter(),
        found.songs.iter(),
        &found.album_tracks,
        (artist_offset, artist_count),
        (album_offset, album_count),
        (song_offset, song_count),
    ))
}

/// Renders a `searchResult3` from already-selected entities.
///
/// `album_tracks` is what `album_node` derives songCount and duration from, and
/// it is deliberately not the matching songs: an album must report its own size,
/// not how much of it the query happened to hit.
#[allow(clippy::too_many_arguments)]
fn search_result<'a>(
    artists: impl Iterator<Item = &'a ArtistItem>,
    albums: impl Iterator<Item = &'a AlbumItem>,
    songs: impl Iterator<Item = &'a SongItem>,
    album_tracks: &[SongItem],
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
        .children(
            albums
                .skip(album_offset)
                .take(album_count)
                .map(|album| album_node(album, album_tracks)),
        )
        .children(songs.skip(song_offset).take(song_count).map(song_node))
}

async fn playlists(state: &AppState, principal: &Principal) -> Result<Node, ProtocolError> {
    let playlists = state
        .services
        .playlists(principal.id)
        .await
        .map_err(internal)?;
    Ok(Node::new("playlists").children(playlists.iter().map(playlist_node)))
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
    Ok(playlist_node(&playlist).children(
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
            // The Subsonic contract is frozen: it has no way to ask for a
            // field to be blanked, so clearing stays off on this surface.
            .update_playlist(
                principal.id,
                id,
                None,
                None,
                None,
                &ids,
                &[],
                Default::default(),
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
    Ok(playlist_node(&playlist).children(
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
    Ok(playlist_node(&playlist))
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
    let snapshot = snapshot(state, principal, params).await?;
    let ids = state
        .services
        .starred_ids(principal.id)
        .await
        .map_err(internal)?;
    let mut node = Node::new("starred2");
    for (kind, id, at) in ids {
        match kind.as_str() {
            "track" => {
                if let Some(item) = snapshot.songs.iter().find(|item| item.id == id) {
                    node.children
                        .push(song_node(item).attr("starred", iso_time(at)));
                }
            }
            "album" => {
                if let Some(item) = snapshot.albums.iter().find(|item| item.id == id) {
                    node.children
                        .push(album_node(item, &snapshot.songs).attr("starred", iso_time(at)));
                }
            }
            "artist" => {
                if let Some(item) = snapshot.artists.iter().find(|item| item.id == id) {
                    node.children
                        .push(artist_node(item, 0).attr("starred", iso_time(at)));
                }
            }
            _ => {}
        }
    }
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
                .map(|share| share_node(share, state.public_url.as_deref())),
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
            Ok(Node::new("shares").child(share_node(&share, state.public_url.as_deref())))
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
            Ok(Node::new("shares").child(share_node(&share, state.public_url.as_deref())))
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
            status: StatusCode::FORBIDDEN,
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
}

fn album_node(album: &AlbumItem, songs: &[SongItem]) -> Node {
    let children = songs
        .iter()
        .filter(|song| song.album_id == Some(album.id))
        .collect::<Vec<_>>();
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
        .attr("songCount", children.len() as i64)
        .attr(
            "duration",
            children
                .iter()
                .map(|song| song.duration_ms / 1000)
                .sum::<i64>(),
        )
        .attr("created", iso_time(album.created_at))
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
        .maybe_attr("starred", song.starred_at.map(iso_time))
        .maybe_attr("userRating", song.user_rating)
        .attr("created", iso_time(song.created_at))
}

fn playlist_node(playlist: &PlaylistItem) -> Node {
    Node::new("playlist")
        .attr("id", playlist.id.to_string())
        .attr("name", playlist.name.clone())
        .maybe_attr("comment", playlist.comment.clone())
        .attr("owner", "")
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

fn share_node(share: &crate::services::ShareItem, public_url: Option<&str>) -> Node {
    let url = share.url_token.as_ref().map(|token| {
        let path = format!("/share/{token}");
        external_url(public_url, &path)
    });
    Node::new("share")
        .attr("id", share.id.to_string())
        .maybe_attr("url", url)
        .maybe_attr("description", share.description.clone())
        .maybe_attr("expires", share.expires_at.map(iso_time))
        .attr("username", "")
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

fn render_protocol(node: Node, json: bool, status: StatusCode) -> Response {
    if json {
        let mut root = Map::new();
        root.insert(node.name.clone(), node_json(&node));
        (
            status,
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            Value::Object(root).to_string(),
        )
            .into_response()
    } else {
        (
            status,
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
            | ("searchResult3", "artist" | "album" | "song")
            | ("playlists", "playlist")
            | ("playlist", "entry")
            | ("starred2", "artist" | "album" | "song")
            | ("nowPlaying", "song")
            | ("playQueue", "song")
            | ("shares", "share")
            | ("share", "entry")
            | ("users", "user")
            | ("user", "folder")
            | ("openSubsonicExtensions", "openSubsonicExtension")
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

fn wants_json_query(query: &str) -> bool {
    parse_pairs(query)
        .ok()
        .and_then(|params| params.first("f").map(str::to_owned))
        .as_deref()
        == Some("json")
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
fn split_multi(value: Option<&str>) -> Vec<String> {
    value
        .into_iter()
        .flat_map(|value| value.split(';'))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
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
        status: StatusCode::UNAUTHORIZED,
    }
}
fn missing() -> ProtocolError {
    ProtocolError {
        code: 10,
        message: "Required parameter is missing",
        status: StatusCode::BAD_REQUEST,
    }
}
fn invalid(message: &'static str) -> ProtocolError {
    ProtocolError {
        code: 10,
        message,
        status: StatusCode::BAD_REQUEST,
    }
}
fn not_found() -> ProtocolError {
    ProtocolError {
        code: 70,
        message: "The requested data was not found",
        status: StatusCode::NOT_FOUND,
    }
}
fn internal(error: impl std::fmt::Display) -> ProtocolError {
    tracing::error!(error = %error, "Subsonic service failure");
    ProtocolError {
        code: 0,
        message: "Internal server error",
        status: StatusCode::INTERNAL_SERVER_ERROR,
    }
}
fn service_protocol(error: ServiceError) -> ProtocolError {
    match error {
        ServiceError::NotFound => not_found(),
        ServiceError::Forbidden => ProtocolError {
            code: 50,
            message: "User is not authorized for the given operation",
            status: StatusCode::FORBIDDEN,
        },
        ServiceError::Invalid => invalid("Invalid parameters"),
        ServiceError::Conflict => ProtocolError {
            code: 0,
            message: "Conflict",
            status: StatusCode::CONFLICT,
        },
        other => internal(other),
    }
}
