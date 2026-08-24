//! Browsing, listing and searching the catalogue.
//!
//! Split out of `subsonic.rs`; the wire contract is frozen, so this moved nothing.

use super::*;

/// Folders, artists and albums for the requested libraries.
///
/// Preferred over [`crate::services::DomainServices::catalog_snapshot`]
/// wherever the answer does not contain tracks: the track read is the
/// expensive third of a snapshot, and since the OpenSubsonic fields landed it
/// carries two relation loads of its own.
pub(super) async fn overview(
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

pub(super) async fn indexes(
    state: &AppState,
    principal: &Principal,
    params: &Params,
    id3: bool,
) -> Result<Node, ProtocolError> {
    let overview = overview(state, principal, params).await?;
    let mut groups: BTreeMap<char, Vec<ArtistSummary>> = BTreeMap::new();
    for artist in overview.artists {
        let initial = artist
            .artist
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
                    // The count comes from the projection now. Filtering every
                    // album for every artist was a loop the facade had no
                    // business running, and it could only ever see the album's
                    // first credit.
                    artist_node(&artist.artist, artist.album_count as usize)
                }))
        })))
}

pub(super) async fn get_artist(
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

pub(super) async fn artist_info(
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
pub(super) async fn album_info(
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

pub(super) async fn get_album(
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

pub(super) async fn get_song(
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

/// Parameter adapter over [`crate::services::DomainServices::list_genres`].
pub(super) async fn genres(
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
pub(super) fn directory_child(node: Node) -> Node {
    node.renamed("child")
        .attr("isDir", true)
        .without("musicBrainzId")
}

pub(super) async fn music_directory(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let id = params.uuid("id")?;
    let overview = overview(state, principal, params).await?;
    let mut directory = Node::new("directory").attr("id", id.to_string());
    if let Some(folder) = overview.folders.iter().find(|item| item.id == id) {
        // The artists of the library, then the tracks that belong to no album.
        // Those name this folder as their `parent` for want of an album id, so
        // this is the level that has to answer for them — otherwise a track
        // advertises a parent that does not contain it.
        let orphans = state
            .services
            .songs_without_album(principal.id, id, MAX_DIRECTORY_SONGS)
            .await
            .map_err(service_protocol)?;
        if orphans.len() as i64 == MAX_DIRECTORY_SONGS {
            tracing::warn!(
                library_id = %id,
                limit = MAX_DIRECTORY_SONGS,
                "album-less tracks reached the folder ceiling; the listing may be short"
            );
        }
        directory = directory
            .attr("name", folder.name.clone())
            .children(
                overview
                    .artists
                    .iter()
                    .filter(|artist| artist.artist.library_id == id)
                    .map(|artist| {
                        directory_child(artist_node(&artist.artist, artist.album_count as usize))
                    }),
            )
            .children(orphans.iter().map(|song| song_node(song).renamed("child")));
    } else if let Some(credited) = match state.services.artist(principal.id, id).await {
        Ok(credited) => Some(credited),
        // Only an absence justifies trying the next branch. A database
        // failure has to say so rather than turn into a not-found, which is
        // the answer this method gives an identifier that does not exist.
        Err(ServiceError::NotFound) => None,
        Err(error) => return Err(service_protocol(error)),
    } {
        // The albums this artist is credited to, by the same rule `getArtist`
        // uses — and resolved the same way, rather than from the overview.
        // The overview lists only artists an album is credited to, so looking
        // the identifier up there would answer 404 for a composer that
        // `getArtist` answers for. Tenancy is unchanged: the service blurs a
        // foreign identifier into the same not-found this arm falls through to.
        directory = directory
            .attr("name", credited.artist.name.clone())
            .children(
                credited
                    .albums
                    .iter()
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
pub(super) async fn album_list(
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

pub(super) async fn random_songs(
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

pub(super) async fn songs_by_genre(
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

pub(super) async fn search(
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
pub(super) fn search_result<'a>(
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
