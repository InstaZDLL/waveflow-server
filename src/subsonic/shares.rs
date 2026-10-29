//! Share methods.
//!
//! Split out of `subsonic.rs`. Two corrections since the contract was frozen,
//! recorded in `docs/compatibility/subsonic-compatibility.md`: every share these methods
//! answer carries its `url`, and `createShare` takes an album's id as well as
//! a track's.

use super::*;

pub(super) async fn shares(
    state: &AppState,
    principal: &Principal,
    method: &str,
    params: &Params,
) -> Result<Node, ProtocolError> {
    match method {
        "getShares" => {
            let mut shares = state
                .services
                .shares(principal.id)
                .await
                .map_err(internal)?;
            for share in &mut shares {
                state.services.reveal_share_link(principal.id, share);
            }
            Ok(Node::new("shares").children(
                shares.iter().map(|share| {
                    share_node(share, &principal.username, state.public_url.as_deref())
                }),
            ))
        }
        "createShare" => {
            let share = state
                .services
                .create_share_of_items(
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
            let mut share = state
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
            state.services.reveal_share_link(principal.id, &mut share);
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
