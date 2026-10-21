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
