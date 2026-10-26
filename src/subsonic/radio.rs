//! Internet radio station methods.
//!
//! The one container in this facade that had never held anything. An empty
//! `internetRadioStations` says "zero stations", which is also what a server
//! without the feature answers — every other absence here declares a role
//! `false` and is therefore legible as an absence. These four methods are what
//! makes that container mean what it says.
//!
//! **Two deliberate deviations from the reference server**, both recorded in
//! `docs/subsonic-compatibility.md`:
//!
//! 1. The list is **per account**, not global. The reference server keeps one
//!    list and lets only the administrator change it; WaveFlow has no global
//!    library setting of any kind, and a per-account list needs no new
//!    privilege to be safe. A client built for the other model sees its own
//!    stations and can edit them, which is the shape it expects — it simply
//!    does not see another account's.
//! 2. `homePageUrl` is spelled with a capital `P` on the way out and
//!    `homepageUrl` without one on the way in, because that is what the
//!    specification does. The asymmetry is the protocol's, not a slip, and
//!    accepting only one spelling would break whichever clients follow the
//!    document.

use super::*;

pub(super) async fn radio_stations(
    state: &AppState,
    principal: &Principal,
    method: &str,
    params: &Params,
) -> Result<Node, ProtocolError> {
    match method {
        "getInternetRadioStations" => Ok(Node::new("internetRadioStations").children(
            state
                .services
                .radio_stations(principal.id)
                .await
                .map_err(internal)?
                .iter()
                .map(station_node),
        )),
        // The three writes answer the bare envelope, as the specification says:
        // they are listed in `empty_success_method`, so a client gets `ok` and
        // reads the list back rather than parsing a station out of a response
        // that was never documented to carry one.
        "createInternetRadioStation" => {
            state
                .services
                .create_radio_station(
                    principal.id,
                    params.first("name").ok_or_else(missing)?,
                    params.first("streamUrl").ok_or_else(missing)?,
                    params.first("homepageUrl"),
                    // The protocol has no image parameter: a station created
                    // here has none, and one edited here keeps its own.
                    None,
                )
                .await
                .map_err(service_protocol)?;
            Ok(Node::new("createInternetRadioStation"))
        }
        "updateInternetRadioStation" => {
            state
                .services
                .update_radio_station(
                    principal.id,
                    params.uuid("id")?,
                    params.first("name").ok_or_else(missing)?,
                    params.first("streamUrl").ok_or_else(missing)?,
                    params.first("homepageUrl"),
                    crate::services::StationImage::Keep,
                )
                .await
                .map_err(service_protocol)?;
            Ok(Node::new("updateInternetRadioStation"))
        }
        "deleteInternetRadioStation" => {
            state
                .services
                .delete_radio_station(principal.id, params.uuid("id")?)
                .await
                .map_err(service_protocol)?;
            Ok(Node::new("deleteInternetRadioStation"))
        }
        _ => unreachable!("radio method dispatch is exhaustive"),
    }
}
