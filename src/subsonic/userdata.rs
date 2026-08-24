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
    Ok(Node::new("bookmarks").children(
        bookmarks
            .iter()
            .map(|bookmark| bookmark_node(bookmark, &principal.username)),
    ))
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

pub(super) async fn set_star(
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
            song_node(song).attr("username", username.clone()).attr(
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
        return Ok(Node::new("playQueue"));
    };
    Ok(Node::new("playQueue")
        .maybe_attr("current", queue.current.map(|id| id.to_string()))
        .attr("position", queue.position_ms)
        .maybe_attr("changedBy", queue.changed_by)
        .attr("changed", iso_time(queue.updated_at))
        .children(queue.songs.iter().map(song_node)))
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
