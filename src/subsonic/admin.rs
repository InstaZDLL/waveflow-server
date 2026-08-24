//! User administration and scan control.
//!
//! Split out of `subsonic.rs`; the wire contract is frozen, so this moved nothing.

use super::*;

pub(super) async fn admin(
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

/// Rescans every library the account can reach.
///
/// Subsonic has no library parameter here, so this fans out. Both the
/// authorization and the queuing live in
/// [`crate::services::DomainServices::start_visible_scans`], so this facade
/// and the native per-library endpoint cannot disagree about who may scan
/// what.
pub(super) async fn start_scan(
    state: &AppState,
    principal: &Principal,
) -> Result<Node, ProtocolError> {
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
pub(super) async fn scan_status(
    state: &AppState,
    principal: &Principal,
) -> Result<Node, ProtocolError> {
    let (scanning, count) = state
        .db
        .scan_progress_for_user(principal.id)
        .await
        .map_err(internal)?;
    Ok(Node::new("scanStatus")
        .attr("scanning", scanning)
        .attr("count", count))
}
