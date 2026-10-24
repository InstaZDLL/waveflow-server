//! Albums, artists, genres and search.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;

#[derive(Debug, Deserialize)]
pub struct BrowseQuery {
    pub library_id: Option<Uuid>,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

/// Album discovery parameters. `sort` accepts the same vocabulary as the
/// Subsonic `type` parameter — both surfaces resolve to [`AlbumOrder`], so the
/// web client can build a home screen ("recently added", "most played") in one
/// call instead of paging the whole catalogue and sorting locally.
#[derive(Debug, Deserialize)]
pub struct AlbumBrowseQuery {
    pub library_id: Option<Uuid>,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
    pub sort: Option<String>,
    /// Required by `sort=byGenre`, ignored otherwise.
    pub genre: Option<String>,
    pub from_year: Option<i64>,
    pub to_year: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct GenreQuery {
    pub library_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    /// Narrows all three kinds to one library, as it does on every other
    /// catalogue route. Absent means every library the account can see.
    pub library_id: Option<Uuid>,
    /// Applied to every kind unless the per-kind offset below overrides it.
    pub offset: Option<i64>,
    pub limit: Option<i64>,
    pub artist_offset: Option<i64>,
    pub album_offset: Option<i64>,
    pub song_offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RandomSongQuery {
    pub library_id: Option<Uuid>,
    pub genre: Option<String>,
    pub from_year: Option<i64>,
    pub to_year: Option<i64>,
    pub limit: Option<i64>,
}

/// The same shape as [`AlbumBrowseQuery`], deliberately: one catalogue should
/// not take two spellings of the same question.
#[derive(Debug, Deserialize)]
pub struct SongBrowseQuery {
    pub library_id: Option<Uuid>,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
    pub sort: Option<String>,
    /// Required by `sort=byGenre`, ignored otherwise.
    pub genre: Option<String>,
    pub from_year: Option<i64>,
    pub to_year: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct TopSongQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct GenreSongQuery {
    pub genre: String,
    pub library_id: Option<Uuid>,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

#[utoipa::path(get, path = "/api/v2/albums", tag = "catalog", params(("library_id" = Option<Uuid>, Query), ("offset" = Option<i64>, Query), ("limit" = Option<i64>, Query), ("sort" = Option<String>, Query), ("genre" = Option<String>, Query), ("from_year" = Option<i64>, Query), ("to_year" = Option<i64>, Query)), responses((status = 200, body = [crate::services::AlbumItem]), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn list_albums(
    State(state): State<AppState>,
    Query(query): Query<AlbumBrowseQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::AlbumItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let order = query
        .sort
        .as_deref()
        .map(crate::services::AlbumOrder::from_str)
        .transpose()
        .map_err(service_error)?
        .unwrap_or_default();
    let request = crate::services::AlbumListQuery {
        library_ids: query.library_id.into_iter().collect(),
        order,
        genre: query.genre,
        from_year: query.from_year,
        to_year: query.to_year,
        page: crate::services::BrowsePage::new(query.offset, query.limit).map_err(service_error)?,
    };
    state
        .services
        .list_albums(user.id, &request)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(get, path = "/api/v2/genres", tag = "catalog", params(("library_id" = Option<Uuid>, Query)), responses((status = 200, body = [crate::services::GenreItem]), (status = 401, body = ErrorResponse)))]
pub async fn list_genres(
    State(state): State<AppState>,
    Query(query): Query<GenreQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::GenreItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let libraries = query.library_id.into_iter().collect::<Vec<_>>();
    state
        .services
        .list_genres(user.id, &libraries)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(get, path = "/api/v2/albums/{album_id}", tag = "catalog", params(("album_id" = Uuid, Path)), responses((status = 200, body = crate::services::AlbumDetail), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn get_album(
    State(state): State<AppState>,
    Path(album_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::services::AlbumDetail>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .album(user.id, album_id)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(get, path = "/api/v2/artists", tag = "catalog", params(("library_id" = Option<Uuid>, Query), ("offset" = Option<i64>, Query), ("limit" = Option<i64>, Query)), responses((status = 200, body = [crate::services::ArtistSummary]), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn list_artists(
    State(state): State<AppState>,
    Query(query): Query<BrowseQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::ArtistSummary>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let page =
        crate::services::BrowsePage::new(query.offset, query.limit).map_err(service_error)?;
    state
        .services
        .list_artists(user.id, query.library_id, page)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(get, path = "/api/v2/artists/{artist_id}", tag = "catalog", params(("artist_id" = Uuid, Path)), responses((status = 200, body = crate::services::ArtistDetail), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn get_artist(
    State(state): State<AppState>,
    Path(artist_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::services::ArtistDetail>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .artist_page(user.id, artist_id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// The albums credited to an artist, as album artist, that this account can see.
///
/// The albums [`get_artist`] carries, without the rest of its page. That page
/// is the artist's own screen: it asks the enrichment engine, which schedules
/// a fill from every configured provider whose row is missing or stale. An
/// album page that only wants the artist's other records would otherwise send
/// a request to a third party for every album opened — or page through the
/// whole library to keep six of them.
#[utoipa::path(get, path = "/api/v2/artists/{artist_id}/albums", tag = "catalog", params(("artist_id" = Uuid, Path)), responses((status = 200, body = [crate::services::AlbumItem]), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn list_artist_albums(
    State(state): State<AppState>,
    Path(artist_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::AlbumItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .artist(user.id, artist_id)
        .await
        .map(|detail| Json(detail.albums))
        .map_err(service_error)
}

/// An artist's most played tracks, for this account.
///
/// Its own route rather than a field on [`get_artist`]: the artist page draws
/// its albums from one answer the cache already holds, and the facade's
/// `getTopSongs` asks this question alone. A field would have made both pay for
/// the other.
///
/// `limit` defaults to ten, which is a screenful on the artist page; the
/// service refuses anything above the catalogue ceiling.
#[utoipa::path(get, path = "/api/v2/artists/{artist_id}/top-songs", tag = "catalog", params(("artist_id" = Uuid, Path), ("limit" = Option<i64>, Query)), responses((status = 200, body = [crate::services::SongItem]), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn list_artist_top_songs(
    State(state): State<AppState>,
    Path(artist_id): Path<Uuid>,
    Query(query): Query<TopSongQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::SongItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .artist_top_songs(user.id, artist_id, query.limit.unwrap_or(10))
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(get, path = "/api/v2/search", tag = "catalog", params(("q" = String, Query), ("library_id" = Option<Uuid>, Query), ("offset" = Option<i64>, Query), ("limit" = Option<i64>, Query), ("artist_offset" = Option<i64>, Query), ("album_offset" = Option<i64>, Query), ("song_offset" = Option<i64>, Query)), responses((status = 200, body = crate::services::SearchResult), (status = 400, description = "q is required"), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn search_catalog(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
    headers: HeaderMap,
) -> Result<Json<crate::services::SearchResult>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    // One offset for all three kinds unless the caller names one, which is
    // what `search3` has always allowed and what a client paging songs past
    // the end of the artists needs.
    let page = |offset: Option<i64>| {
        crate::services::BrowsePage::new(offset.or(query.offset), query.limit)
            .map_err(service_error)
    };
    let libraries = query.library_id.into_iter().collect::<Vec<_>>();
    state
        .services
        .search(
            user.id,
            &query.q,
            &libraries,
            page(query.artist_offset)?,
            page(query.album_offset)?,
            page(query.song_offset)?,
        )
        .await
        .map(Json)
        .map_err(service_error)
}

/// The catalogue's songs, paged and sorted.
///
/// **The listing that did not exist.** #179 reported this path requiring a
/// `genre` and so listing nothing; it was closed on 2026-09-10 by renaming the
/// route to `/songs/by-genre`, which made the contract honest without making
/// the listing exist. What a client had instead was
/// `/libraries/{library_id}/tracks`: it pages and searches, but takes one
/// library at a time, answers the thinner `TrackRecord`, and does not sort.
///
/// `/songs/by-genre` stays where it is. It is the native form of
/// `getSongsByGenre`, which the facade builds on, and `genre` is required
/// there on purpose. Here a genre is one of eleven orderings, the same way it
/// is for albums.
#[utoipa::path(get, path = "/api/v2/songs", tag = "catalog", params(("library_id" = Option<Uuid>, Query), ("offset" = Option<i64>, Query), ("limit" = Option<i64>, Query), ("sort" = Option<String>, Query), ("genre" = Option<String>, Query), ("from_year" = Option<i64>, Query), ("to_year" = Option<i64>, Query)), responses((status = 200, body = [crate::services::SongItem]), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn list_songs(
    State(state): State<AppState>,
    Query(query): Query<SongBrowseQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::SongItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let order = query
        .sort
        .as_deref()
        .map(crate::services::SongOrder::from_str)
        .transpose()
        .map_err(service_error)?
        .unwrap_or_default();
    let request = crate::services::SongListQuery {
        library_ids: query.library_id.into_iter().collect(),
        order,
        genre: query.genre,
        from_year: query.from_year,
        to_year: query.to_year,
        page: crate::services::BrowsePage::new(query.offset, query.limit).map_err(service_error)?,
    };
    state
        .services
        .list_songs(user.id, &request)
        .await
        .map(Json)
        .map_err(service_error)
}

/// The native form of `getRandomSongs`.
///
/// The selection is drawn in SQL, so a request for ten reads ten. `genre`
/// matches the canonical name, like every other genre filter on either
/// surface, and a reversed year range is read as a range rather than as an
/// empty one.
#[utoipa::path(get, path = "/api/v2/songs/random", tag = "catalog", params(("library_id" = Option<Uuid>, Query), ("genre" = Option<String>, Query), ("from_year" = Option<i64>, Query), ("to_year" = Option<i64>, Query), ("limit" = Option<i64>, Query)), responses((status = 200, body = [crate::services::SongItem]), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn list_random_songs(
    State(state): State<AppState>,
    Query(query): Query<RandomSongQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::SongItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .random_songs(
            user.id,
            query.library_id.as_slice(),
            query.genre.as_deref(),
            query.from_year,
            query.to_year,
            query.limit.unwrap_or(10),
        )
        .await
        .map(Json)
        .map_err(service_error)
}

/// The native form of `getSongsByGenre`. `genre` is required: answering an
/// unfiltered catalogue would drop the filter in silence.
///
/// Served at `/songs/by-genre` and not `/songs`, so the path says what the
/// handler does. Under `/songs` it read as the general listing and answered
/// 400 to anyone who took it for one; the general listing is
/// `/libraries/{id}/tracks`, which pages and searches.
#[utoipa::path(get, path = "/api/v2/songs/by-genre", tag = "catalog", params(("genre" = String, Query), ("library_id" = Option<Uuid>, Query), ("offset" = Option<i64>, Query), ("limit" = Option<i64>, Query)), responses((status = 200, body = [crate::services::SongItem]), (status = 400, description = "genre is required"), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn list_songs_by_genre(
    State(state): State<AppState>,
    Query(query): Query<GenreSongQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::SongItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let page =
        crate::services::BrowsePage::new(query.offset, query.limit).map_err(service_error)?;
    state
        .services
        .songs_by_genre(user.id, query.library_id.as_slice(), &query.genre, page)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Chooses an artist's `portrait` or `banner` from an image's bytes
/// (RFC-013): JPEG, PNG or WebP, at most 10 MiB, recognised from the bytes.
/// Owners and managers; anyone else gets the same `404` as a stranger.
#[utoipa::path(put, path = "/api/v2/artists/{artist_id}/image/{kind}", tag = "catalog", params(("artist_id" = Uuid, Path), ("kind" = String, Path, description = "portrait or banner")), request_body(content = Vec<u8>, content_type = "application/octet-stream"), responses((status = 200, body = crate::services::ArtistDetail), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 413, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn put_artist_image(
    State(state): State<AppState>,
    Path((artist_id, kind)): Path<(Uuid, String)>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<crate::services::ArtistDetail>, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let kind = crate::services::ArtistImageKind::parse(&kind).ok_or(ApiError::NotFound)?;
    state
        .services
        .choose_artist_image(user.id, artist_id, kind, body.to_vec())
        .await
        .map_err(service_error)?;
    state
        .services
        .artist_page(user.id, artist_id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Goes back to the automatic image: the portrait the files or a provider
/// gave, or no banner. `404` when nothing was chosen.
#[utoipa::path(delete, path = "/api/v2/artists/{artist_id}/image/{kind}", tag = "catalog", params(("artist_id" = Uuid, Path), ("kind" = String, Path, description = "portrait or banner")), responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn delete_artist_image(
    State(state): State<AppState>,
    Path((artist_id, kind)): Path<(Uuid, String)>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let kind = crate::services::ArtistImageKind::parse(&kind).ok_or(ApiError::NotFound)?;
    state
        .services
        .remove_artist_image(user.id, artist_id, kind)
        .await
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(service_error)
}

#[derive(Debug, Deserialize)]
pub struct CandidateQuery {
    pub q: String,
}

/// Deezer's artists for `q`, offered as this artist's portrait (RFC-013): up
/// to 20, each with a preview the server fetched and holds, and no picture
/// URL. Stored as a search for an hour, and the same query answered from it
/// while it lives. Owners and managers; `404` for anyone else and where
/// Deezer is not configured; `429` while the account's previous search is
/// still fetching; `503` when Deezer could not be reached.
#[utoipa::path(get, path = "/api/v2/artists/{artist_id}/image/candidates", tag = "catalog", params(("artist_id" = Uuid, Path), ("q" = String, Query)), responses((status = 200, body = [crate::services::ArtistImageCandidate]), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse), (status = 429, body = ErrorResponse), (status = 503, body = ErrorResponse)))]
pub async fn list_artist_image_candidates(
    State(state): State<AppState>,
    Path(artist_id): Path<Uuid>,
    Query(query): Query<CandidateQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::ArtistImageCandidate>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .artist_image_candidates(user.id, artist_id, &query.q)
        .await
        .map(Json)
        .map_err(service_error)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeezerPortraitPick {
    /// A Deezer artist id offered by a live search of this artist.
    pub deezer_id: i64,
}

/// Chooses as the portrait the preview a live search of this artist offered
/// for `deezer_id`, never fetched again. `422` for an id no live search of
/// this artist offered; `404` where Deezer is not configured now.
#[utoipa::path(put, path = "/api/v2/artists/{artist_id}/image/portrait/deezer", tag = "catalog", params(("artist_id" = Uuid, Path)), request_body = DeezerPortraitPick, responses((status = 200, body = crate::services::ArtistDetail), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn put_artist_deezer_portrait(
    State(state): State<AppState>,
    Path(artist_id): Path<Uuid>,
    headers: HeaderMap,
    Json(pick): Json<DeezerPortraitPick>,
) -> Result<Json<crate::services::ArtistDetail>, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .choose_deezer_portrait(user.id, artist_id, pick.deezer_id)
        .await
        .map_err(service_error)?;
    state
        .services
        .artist_page(user.id, artist_id)
        .await
        .map(Json)
        .map_err(service_error)
}
