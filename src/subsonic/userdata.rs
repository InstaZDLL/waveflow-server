//! Bookmarks, favourites, ratings, scrobbles and the queue.
//!
//! Split out of `subsonic.rs`; the wire contract is frozen, so this moved nothing.

use super::*;

pub(super) async fn bookmarks(
    state: &AppState,
    principal: &Principal,
) -> Result<Node, ProtocolError> {
    let bookmarks = state
        .services
        .bookmarks(principal.id)
        .await
        .map_err(internal)?;
    Ok(Node::new("bookmarks")
        .children(
            bookmarks
                .iter()
                .map(|bookmark| bookmark_node(bookmark, &principal.username)),
        )
        // Positions in podcast episodes, after the tracks (RFC-025).
        .children(episode_bookmark_nodes(state, principal).await?))
}

pub(super) async fn create_bookmark(
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

pub(super) async fn delete_bookmark(
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

/// How many identifiers one request may name, across every parameter that
/// carries them.
///
/// Each one costs a mutation that takes the process-wide writer gate, and the
/// form body admits some fifteen hundred UUIDs — enough for one well-formed
/// request to hold the gate against every other writer for as long as it takes
/// to serialise them. The bound is the one shares use, and for the same reason;
/// it was the queue's too until the queue was raised to hold a whole playlist,
/// which it writes in one statement rather than one mutation per track.
const MAX_IDENTIFIERS: usize = crate::services::MAX_REQUEST_IDENTIFIERS;

pub(super) async fn set_star(
    state: &AppState,
    principal: &Principal,
    params: &Params,
    starred: bool,
) -> Result<Node, ProtocolError> {
    // Applied together or not at all (`set_stars`): a request refused halfway
    // left the first few starred and reported an error for the whole, and a
    // client retrying it starred them again.
    let tracks = params.uuids("id")?;
    let albums = params.uuids("albumId")?;
    let artists = params.uuids("artistId")?;
    if tracks.len() + albums.len() + artists.len() > MAX_IDENTIFIERS {
        return Err(invalid("Too many identifiers in one request"));
    }
    let mut targets = Vec::with_capacity(tracks.len() + albums.len() + artists.len());
    for id in tracks {
        let kind = state
            .services
            .entity_kind(principal.id, id)
            .await
            .map_err(service_protocol)?
            .ok_or_else(not_found)?;
        targets.push((kind, id));
    }
    targets.extend(albums.into_iter().map(|id| ("album", id)));
    targets.extend(artists.into_iter().map(|id| ("artist", id)));
    state
        .services
        .set_stars(principal.id, &targets, starred)
        .await
        .map_err(service_protocol)?;
    Ok(Node::new(if starred { "star" } else { "unstar" }))
}

pub(super) async fn starred(
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

pub(super) async fn set_rating(
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

pub(super) async fn scrobble(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let ids = params.uuids("id")?;
    if ids.is_empty() {
        return Err(missing());
    }
    if ids.len() > MAX_IDENTIFIERS {
        return Err(invalid("Too many identifiers in one request"));
    }
    let times = params
        .all("time")
        .iter()
        .map(|value| value.parse::<i64>().map_err(|_| invalid("Invalid time")))
        .collect::<Result<Vec<_>, _>>()?;
    let submission = params.bool_optional("submission")?.unwrap_or(true);
    // Recorded together or not at all (`scrobble_all`), for the reason
    // `set_star` gives.
    let plays = ids
        .into_iter()
        .enumerate()
        .map(|(index, id)| (id, times.get(index).copied()))
        .collect::<Vec<_>>();
    state
        .services
        .scrobble_all(principal.id, &plays, submission)
        .await
        .map_err(service_protocol)?;
    Ok(Node::new("scrobble"))
}

pub(super) async fn now_playing(
    state: &AppState,
    principal: &Principal,
) -> Result<Node, ProtocolError> {
    let entries = state
        .services
        .now_playing(principal.id)
        .await
        .map_err(internal)?;
    Ok(
        Node::new("nowPlaying").children(entries.iter().map(|(username, song, started)| {
            // `entry`, not `song`: the schema renames a media item inside this
            // container exactly as it does inside a playlist.
            song_node(song)
                .renamed("entry")
                .attr("username", username.clone())
                .attr(
                    "minutesAgo",
                    ((chrono::Utc::now().timestamp_millis() - started) / 60_000).max(0),
                )
        })),
    )
}

pub(super) async fn get_queue(
    state: &AppState,
    principal: &Principal,
) -> Result<Node, ProtocolError> {
    let Some(queue) = state.services.queue(principal.id).await.map_err(internal)? else {
        // Nothing has been saved, so there is no `playQueue` at all: the
        // schema marks `username`, `changed` and `changedBy` all three
        // required, which leaves the element no valid form to take when the
        // queue has never been written. The reference documents the same
        // answer — "an empty <subsonic-response> if no play queue has been
        // saved" — and omission is what a client built against it already
        // handles. Filling the element in would mean inventing a `changed`
        // and a `changedBy` for something that was never changed (#229).
        return Ok(Node::nothing());
    };
    Ok(Node::new("playQueue")
        .attr("username", principal.username.clone())
        .maybe_attr("current", queue.current.map(|id| id.to_string()))
        .attr("position", queue.position_ms)
        .maybe_attr("changedBy", queue.changed_by)
        .attr("changed", iso_time(queue.updated_at))
        // `entry`, not `song` — see `nowPlaying` above.
        .children(
            queue
                .songs
                .iter()
                .map(|song| song_node(song).renamed("entry")),
        ))
}

pub(super) async fn save_queue(
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
