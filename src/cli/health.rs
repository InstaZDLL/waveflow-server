//! `health`: ask a running server whether it is ready, for a container's
//! `HEALTHCHECK` or a supervisor that has no HTTP client of its own.
//!
//! The image carried curl for that one line, and curl alone brought over a
//! hundred scanner alerts with it (#394, #396). The binary already links an
//! HTTP client, so it asks itself.

use std::time::Duration;

use crate::Config;

/// How long the whole exchange may take. A server that is ready answers
/// `/ready` in milliseconds. Shorter than the image's `HEALTHCHECK --timeout`
/// of five seconds, so the command gives up and says why before Docker kills
/// it and keeps only "exceeded timeout".
const TIMEOUT: Duration = Duration::from_secs(3);

/// Ask `/ready` on the configured bind address and succeed only on the 200
/// that WaveFlow's readiness probe gives.
///
/// Runs on the `Config` alone, before [`crate::initialize`]: a probe must not
/// open, migrate or lock the database of the server it is asking about. Silent
/// when the server is ready, one line when it is not — `docker inspect` keeps
/// that line, and nothing more is needed to tell a refused connection from a
/// 503.
pub async fn health(config: &Config) -> anyhow::Result<()> {
    let url = format!(
        "http://{}/ready",
        crate::app::laptop::reachable(config.bind_addr)
    );
    // No proxy, so a system proxy is never asked about a loopback address, and
    // no redirect, so the answer is the one the port gave.
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(TIMEOUT)
        .build()?;
    let response = match client.get(&url).send().await {
        Ok(response) => response,
        Err(error) if error.is_timeout() => {
            anyhow::bail!("{url} did not answer within {} s", TIMEOUT.as_secs())
        }
        Err(error) => anyhow::bail!("{url} is unreachable: {}", root_cause(&error)),
    };
    let status = response.status();
    if status != reqwest::StatusCode::OK {
        anyhow::bail!("{url} answered {status}");
    }
    // The body too, as the laptop probe checks `/health`'s: any path this
    // server does not route answers 200 with the web client's page, and
    // another program on the port may answer 200 to anything.
    match response.json::<serde_json::Value>().await {
        Ok(body) if body["status"] == "ready" => Ok(()),
        _ => anyhow::bail!("{url} answered 200, but not as WaveFlow's readiness probe"),
    }
}

/// The innermost error of the chain: "connection refused", rather than
/// reqwest's "error sending request" wrapped around it.
fn root_cause(error: &(dyn std::error::Error + 'static)) -> String {
    let mut cause = error;
    while let Some(source) = cause.source() {
        cause = source;
    }
    cause.to_string()
}
