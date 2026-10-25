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
///
/// **`username`** names another account, and is an administrator's (the
/// specification: "The user must be admin in order to use this parameter").
/// An administrator naming someone else is answered that account's own
/// playlists, private ones included, each read through the administrator's
/// libraries. Anyone else naming another account — and anyone naming an
/// account that does not exist — is answered error 70, the not-found every
/// other refusal here gives: a 50 would confirm the name belongs to someone.
/// Naming oneself is the ordinary listing.
pub(super) async fn playlists(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let named = params
        .first("username")
        .map(str::trim)
        .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case(&principal.username));
    let playlists = match named {
        None => state
            .services
            .visible_playlists(principal.id)
            .await
            .map_err(internal)?,
        Some(_) if principal.role != AccountRole::Admin => return Err(not_found()),
        Some(name) => state
            .services
            .administered_playlists(principal.id, name)
            .await
            .map_err(service_protocol)?,
    };
    Ok(Node::new("playlists").children(playlists.iter().map(playlist_node)))
}

pub(super) async fn get_playlist(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Node, ProtocolError> {
    let id = params.uuid("id")?;
    // An administrator opens what `getPlaylists?username=` listed for them;
    // everyone else reads their own and the public ones, as before.
    let playlist = match state.services.playlist(principal.id, id).await {
        Err(crate::services::ServiceError::NotFound) if principal.role == AccountRole::Admin => {
            state.services.administered_playlist(principal.id, id).await
        }
        other => other,
    }
    .map_err(service_protocol)?;
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
