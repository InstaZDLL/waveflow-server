//! Smart playlists on the façade (RFC-025 decision 6): read-only playlists in
//! `getPlaylists` and `getPlaylist`.
//!
//! Kept apart from `playlists.rs`, which carries the ordinary ones, so the two
//! meet at one line each: `getPlaylists` appends these after the others, and
//! `getPlaylist` asks here when no ordinary playlist has the id.
//!
//! Each carries the OpenSubsonic `readonly` attribute, true, and `validUntil`,
//! the moment it was evaluated for this caller: a client that caches it knows
//! the copy it holds is already due. `getPlaylists` evaluates none of them
//! (RFC-025 decision 4); its counts are the caller's last `getPlaylist`. Every write
//! the façade has — `updatePlaylist`, `deletePlaylist`, `createPlaylist` with a
//! `playlistId` — reaches ordinary playlists only, so naming a smart one there
//! answers error 70, as a missing playlist does.

use super::*;

use crate::services::SmartPlaylistItem;

fn smart_playlist_node(playlist: &SmartPlaylistItem) -> Node {
    Node::new("playlist")
        .attr("id", playlist.id.to_string())
        .attr("name", playlist.name.clone())
        .maybe_attr("comment", playlist.comment.clone())
        .attr("owner", playlist.owner_username.clone())
        .attr("public", playlist.public)
        // In `getPlaylists`, what this caller was last shown of it — a listing
        // evaluates nothing — and zero when they have not opened it since the
        // server started; `validUntil` is then absent. In `getPlaylist`, now.
        .attr("songCount", playlist.song_count.unwrap_or(0))
        .attr("duration", playlist.duration_ms.unwrap_or(0) / 1000)
        .attr("created", iso_time(playlist.created_at))
        .attr("changed", iso_time(playlist.updated_at))
        .attr("readonly", true)
        .maybe_attr("validUntil", playlist.evaluated_at.map(iso_time))
}

/// Every smart playlist this account may read: its own, then other accounts'
/// public ones.
pub(super) async fn smart_playlist_nodes(
    state: &AppState,
    principal: &Principal,
) -> Result<Vec<Node>, ProtocolError> {
    Ok(state
        .services
        .smart_playlists(principal.id, true)
        .await
        .map_err(internal)?
        .iter()
        .map(smart_playlist_node)
        .collect())
}

/// One smart playlist and its tracks, evaluated now; error 70 when there is
/// none this account may read.
pub(super) async fn get_smart_playlist(
    state: &AppState,
    principal: &Principal,
    id: Uuid,
) -> Result<Node, ProtocolError> {
    let playlist = state
        .services
        .smart_playlist(principal.id, id)
        .await
        .map_err(service_protocol)?;
    Ok(smart_playlist_node(&playlist).children(
        playlist
            .songs
            .iter()
            .flatten()
            .map(|song| song_node(song).renamed("entry")),
    ))
}
