//! What a server in laptop mode does at boot that an installed one does not
//! (`docs/plans/standalone-download-plan.md`, §5): make sure the port is not
//! already this server's, and tell the person reading the console window what
//! they are looking at.
//!
//! None of it runs unless [`Config::laptop`] is on, so a server under systemd
//! or Docker binds, warns and logs exactly as it did before laptop mode
//! existed.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::time::Duration;

use tokio::net::TcpListener;

use crate::config::LAPTOP_ENV_FILE;
use crate::Config;

/// How the port turned out.
#[derive(Debug)]
pub enum Startup {
    /// The port was free and is now this process's.
    Listening(TcpListener),
    /// Another copy of WaveFlow is already serving on it, at this address.
    /// Double-clicking the server a second time lands here, and the new
    /// process has nothing to do but say so and stop.
    AlreadyRunning(String),
}

/// How long the copy on the port has to answer its health probe. A WaveFlow
/// on the same machine answers in milliseconds; anything slower is not one.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Take the configured port, or find out whose it is.
///
/// Called **before** [`crate::initialize`], and that order is the point: a
/// second copy that opened the database first would run the boot against
/// files the first copy is serving — migrations, a backup, a freshly drawn
/// setup token written over the one the first copy is waiting for.
///
/// The port never moves. A Subsonic client on a phone and the user's bookmark
/// keep the address, so a port that is taken by anything other than WaveFlow
/// is a refusal that says which line of `laptop.env` to change, never a
/// quiet fall-back to another port. Navidrome defaults to 4533 too, so this
/// will happen.
pub async fn bind(config: &Config, laptop_env: Option<&Path>) -> anyhow::Result<Startup> {
    let address = config.bind_addr;
    let error = match TcpListener::bind(address).await {
        Ok(listener) => return Ok(Startup::Listening(listener)),
        Err(error) => error,
    };
    if error.kind() == std::io::ErrorKind::AddrInUse && answers_as_waveflow(address).await {
        return Ok(Startup::AlreadyRunning(browser_url(address)));
    }
    let file = laptop_env.map_or_else(
        || LAPTOP_ENV_FILE.to_owned(),
        |path| path.display().to_string(),
    );
    let next = address.port().checked_add(1).unwrap_or(4534);
    Err(anyhow::anyhow!(
        "WaveFlow Server cannot use port {port} on {ip}, which another program holds ({error}). \
         Choose another port with the WAVEFLOW_BIND line of {file} — for example \
         WAVEFLOW_BIND={ip}:{next} — then start the server again.",
        port = address.port(),
        ip = address.ip(),
    ))
}

/// Whether what holds `address` is a WaveFlow, by its health probe.
///
/// The body is checked, not only the status: another server on 4533 may well
/// answer `/health` with a 200 of its own. No proxy, so a system proxy
/// is never asked about a loopback address, and no redirect, so the answer is
/// the one the port gave.
async fn answers_as_waveflow(address: SocketAddr) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(PROBE_TIMEOUT)
        .build()
    else {
        return false;
    };
    let url = format!("http://{}/health", reachable(address));
    let Ok(response) = client.get(url).send().await else {
        return false;
    };
    if !response.status().is_success() {
        return false;
    }
    response
        .json::<serde_json::Value>()
        .await
        .is_ok_and(|body| is_waveflow_health(&body))
}

/// The body `/health` answers, as `crate::api::probes::health` writes
/// it: `status`, `version` and the schema generation.
fn is_waveflow_health(body: &serde_json::Value) -> bool {
    body["status"] == "ok" && body["schema"] == 2 && body["version"].is_string()
}

/// The address to reach a server bound to `address` from this machine: the
/// loopback of the same family when it listens on every interface.
fn reachable(address: SocketAddr) -> SocketAddr {
    let ip = match address.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    SocketAddr::new(ip, address.port())
}

/// The address a browser on this machine opens.
fn browser_url(address: SocketAddr) -> String {
    format!("http://{}/", reachable(address))
}

/// The one line the console window shows: where to go, where the data is,
/// and how to stop it. Closing the window is how a double-clicked server
/// stops, and nothing else on screen says so.
pub fn banner(address: SocketAddr, data_dir: &Path) -> String {
    let data_dir = std::path::absolute(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    format!(
        "WaveFlow Server is running at {} (data in {}). Close this window to stop the server.",
        browser_url(address),
        data_dir.display()
    )
}

/// What the LAN can use, in place of the warning about `WAVEFLOW_PUBLIC_URL`.
///
/// That warning is aimed at an operator behind a reverse proxy, and is noise
/// on a laptop: a laptop on Wi-Fi has no stable address to name, and leaving
/// the setting unset is what lets a phone browsing to the laptop's address
/// sign in, since the origin check then falls back to the `Host` header.
/// What the person needs instead is the address their phone can use — or, on
/// the default loopback bind, the line that opens it.
pub fn network_hint(address: SocketAddr) -> String {
    let lan = address.ip().is_unspecified().then(lan_address).flatten();
    describe_network(address, lan)
}

fn describe_network(address: SocketAddr, lan: Option<IpAddr>) -> String {
    let port = address.port();
    let ip = address.ip();
    if ip.is_loopback() {
        format!(
            "Only this computer can reach the server. To let a phone or another computer \
             on your network in, set WAVEFLOW_BIND=0.0.0.0:{port} in {LAPTOP_ENV_FILE} and \
             start the server again."
        )
    } else if ip.is_unspecified() {
        match lan {
            Some(lan) => format!(
                "Devices on your network can reach the server at {}",
                browser_url(SocketAddr::new(lan, port))
            ),
            None => format!(
                "Devices on your network can reach the server at this computer's \
                 network address, on port {port}."
            ),
        }
    } else {
        format!(
            "Devices on your network can reach the server at {}",
            browser_url(address)
        )
    }
}

/// This machine's address on the network its default route leads to.
///
/// Connecting a UDP socket sends nothing: it only asks the routing table which
/// interface would carry a datagram. `192.0.2.1` is a documentation address
/// (RFC 5737) that nothing answers, so even a mistake costs no traffic.
fn lan_address() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

#[cfg(test)]
mod tests;
