//! Share methods.
//!
//! Split out of `subsonic.rs`; the wire contract is frozen, so this moved nothing.

use super::*;

pub(super) async fn shares(
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
                .map(|share| share_node(share, &principal.username, state.public_url.as_deref())),
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
            Ok(Node::new("shares").child(share_node(
                &share,
                &principal.username,
                state.public_url.as_deref(),
            )))
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
            Ok(Node::new("shares").child(share_node(
                &share,
                &principal.username,
                state.public_url.as_deref(),
            )))
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
