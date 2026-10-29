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

/// How long the copy on the port has to answer its health probe at first. A
/// WaveFlow that is serving answers in milliseconds.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// How much longer a port that accepted the probe but has not answered it is
/// given. A first copy holds its port from before its boot, so while it
/// migrates, copies its database aside or checks FFmpeg, the kernel accepts
/// connections and nobody reads them; the request queued there is answered
/// as soon as it starts serving. A second double-click is impatient by
/// nature, and lands exactly in that window.
const STARTING_GRACE: Duration = Duration::from_secs(30);

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
    bind_probing(config, laptop_env, PROBE_TIMEOUT, STARTING_GRACE).await
}

/// [`bind`], with the two waits as parameters so a test need not spend them.
async fn bind_probing(
    config: &Config,
    laptop_env: Option<&Path>,
    first: Duration,
    grace: Duration,
) -> anyhow::Result<Startup> {
    let address = config.bind_addr;
    let error = match TcpListener::bind(address).await {
        Ok(listener) => return Ok(Startup::Listening(listener)),
        Err(error) => error,
    };
    let file = laptop_env.map_or_else(
        || LAPTOP_ENV_FILE.to_owned(),
        |path| path.display().to_string(),
    );
    let port = address.port();
    let ip = address.ip();
    // Nobody holds the port: the address itself is wrong for this machine —
    // a LAN address the laptop no longer has after changing networks, or a
    // port the system reserves. Another port is not necessarily the fix, so
    // the refusal says what the line has to name instead.
    if error.kind() != std::io::ErrorKind::AddrInUse {
        anyhow::bail!(
            "WaveFlow Server cannot listen on {address} ({error}). Check the WAVEFLOW_BIND line \
             of {file}: it must name an address of this computer, such as \
             WAVEFLOW_BIND=127.0.0.1:{port} for this computer only or \
             WAVEFLOW_BIND=0.0.0.0:{port} for the network, then start the server again."
        );
    }
    let mut probe = health_probe(address, first).await;
    if probe == Probe::Silent {
        tracing::info!(
            port,
            "the port is held by a program that has not answered yet; waiting in case it is WaveFlow starting up"
        );
        probe = health_probe(address, grace).await;
    }
    if probe == Probe::WaveFlow {
        return Ok(Startup::AlreadyRunning(browser_url(address)));
    }
    let next = port.checked_add(1).unwrap_or(4534);
    let change = format!(
        "choose another port with the WAVEFLOW_BIND line of {file} — for example \
         WAVEFLOW_BIND={ip}:{next} — then start the server again"
    );
    Err(if probe == Probe::Silent {
        anyhow::anyhow!(
            "WaveFlow Server cannot use port {port} on {ip}: the program holding it accepted a \
             connection and did not answer within {} seconds. If it is WaveFlow still starting, \
             wait for it and open {}; otherwise {change}.",
            (first + grace).as_secs(),
            browser_url(address),
        )
    } else {
        anyhow::anyhow!(
            "WaveFlow Server cannot use port {port} on {ip}, which another program holds \
             ({error}). To start this one anyway, {change}."
        )
    })
}

/// What answered on a port that was already taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Probe {
    /// A WaveFlow, serving.
    WaveFlow,
    /// Something that answered, or hung up, as WaveFlow does not.
    Foreign,
    /// Something that took the connection and said nothing before the
    /// timeout: a program that does not speak HTTP, or a WaveFlow whose boot
    /// has not reached its router yet. Only waiting tells them apart.
    Silent,
}

/// Ask `/health` on `address`, waiting at most `timeout` for the answer.
///
/// The body is checked, not only the status: another server on 4533 may well
/// answer `/health` with a 200 of its own. No proxy, so a system proxy
/// is never asked about a loopback address, and no redirect, so the answer is
/// the one the port gave.
async fn health_probe(address: SocketAddr, timeout: Duration) -> Probe {
    let Ok(client) = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
    else {
        return Probe::Foreign;
    };
    let url = format!("http://{}/health", reachable(address));
    let response = match client.get(url).send().await {
        Ok(response) => response,
        Err(error) if error.is_timeout() => return Probe::Silent,
        Err(_) => return Probe::Foreign,
    };
    if !response.status().is_success() {
        return Probe::Foreign;
    }
    match response.json::<serde_json::Value>().await {
        Ok(body) if is_waveflow_health(&body) => Probe::WaveFlow,
        _ => Probe::Foreign,
    }
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
