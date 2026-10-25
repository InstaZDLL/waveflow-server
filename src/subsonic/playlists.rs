//! Playlist methods.
//!
//! Split out of `subsonic.rs`; the wire contract is frozen, so this moved nothing.

use super::*;

/// `getPlaylists`: this account's playlists, then every other account's
/// public ones, each under the name of the account that owns it (RFC-020
/// decision 2).
///
/// What the specification calls "all playlists a user is allowed to play".
/// Until that RFC the list was the caller's own whatever the `public` flag
/// said, so the flag this façade accepted had no effect anywhere. A client
/// reads `owner` to decide what it may edit, which is why the field has to be
/// the owner's and not the caller's; the writes stay refused to anyone else.
pub(super) async fn playlists(
    state: &AppState,
    principal: &Principal,
) -> Result<Node, ProtocolError> {
    let playlists = state
        .services
        .visible_playlists(principal.id)
        .await
        .map_err(internal)?;
    Ok(Node::new("playlists")
        .children(playlists.iter().map(playlist_node))
        // Smart playlists after them, read-only (RFC-025 decision 6).
        .children(smart_playlist_nodes(state, principal).await?))
}

pub(super) async fn get_playlist(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let id = params.uuid("id")?;
    let playlist = match state.services.playlist(principal.id, id).await {
        // No ordinary playlist has the id: perhaps a smart one does (RFC-025).
        Err(ServiceError::NotFound) => return get_smart_playlist(state, principal, id).await,
        other => other.map_err(service_protocol)?,
    };
    Ok(playlist_node(&playlist).children(
        playlist
            .songs
            .iter()
            .map(|song| song_node(song).renamed("entry")),
    ))
}

pub(super) async fn create_playlist(
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
    Ok(playlist_node(&playlist).children(
        playlist
            .songs
            .iter()
            .map(|song| song_node(song).renamed("entry")),
    ))
}

pub(super) async fn update_playlist(
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

pub(super) async fn delete_playlist(
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
