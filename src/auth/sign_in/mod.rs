//! Guarding sign-in against guessing (RFC-017).
//!
//! Three pieces, shared by every place a password is checked:
//!
//! - **The limiter** counts failures per (client address, account) pair, twenty
//!   in a sliding minute, and reserves each attempt *before* it is checked, so
//!   concurrent guesses cannot all slip under the count. A pair, never the
//!   account alone: an attacker elsewhere spends their own budget, not the
//!   owner's.
//! - **The gate** bounds how many Argon2 verifications run at once, and how many
//!   wait for one, so the memory a burst of guesses costs is bounded by the
//!   gate and not by the request rate.
//! - **The tally** counts refusals and logs them at most once a minute, plus the
//!   first throttle, and the first unreadable stored hash, after a quiet minute:
//!   a burst of guesses never becomes a burst of log lines. No line carries a
//!   username, an address or a key.

use std::{
    collections::{HashMap, VecDeque},
    net::IpAddr,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use axum::http::HeaderMap;
use ipnet::IpNet;
use sha2::{Digest, Sha256};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Failures one (address, account) pair may make in a sliding minute.
pub const FAILURES_PER_WINDOW: usize = 20;

/// The sliding window.
const WINDOW: Duration = Duration::from_secs(60);

/// Live windows one address may hold, the overflow window included: 31 for
/// accounts and one reserved for everything past them.
pub const WINDOWS_PER_ADDRESS: usize = 32;

/// Live windows the whole table may hold. Past it, a new key is refused while
/// every window already counting goes on: the limiter fails closed for
/// strangers rather than forget a window that is counting.
pub const MAX_WINDOWS: usize = 65_536;

/// The account an address's attempts share once it has used its own windows.
const OVERFLOW_ACCOUNT: &str = "*";

/// How long a sign-in waits for a verification permit before answering 503.
const PERMIT_WAIT: Duration = Duration::from_secs(5);

/// How many requests may wait for a permit, per permit.
const WAITING_PER_PERMIT: usize = 4;

/// Where a password was presented, for the tally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Native,
    Web,
    Subsonic,
}

/// Why a sign-in was refused, for the tally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The pair's window was full, or the table was.
    Throttled,
    /// A wrong password, an unknown or disabled account, an unknown API key.
    WrongCredentials,
    /// No verification permit: the queue was full or the wait ran out.
    Busy,
    /// The account's stored hash cannot be read. The server's fault, so the
    /// attempt's slot goes back — which is why it is counted here, not logged
    /// per attempt: nothing else would bound a burst against that account.
    UnreadableHash,
}

/// The pair's budget is spent, or the table is full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Throttled;

/// The address a request is counted under: an IPv4 address alone, an IPv6
/// address by its `/64` (the block one host routinely holds), and `unknown`
/// when the server was not told one, which only happens in-process.
pub fn address_bucket(address: Option<IpAddr>) -> String {
    match address {
        None => "unknown".to_owned(),
        Some(IpAddr::V4(v4)) => v4.to_string(),
        Some(IpAddr::V6(v6)) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => {
                let segments = v6.segments();
                format!(
                    "{:x}:{:x}:{:x}:{:x}::/64",
                    segments[0], segments[1], segments[2], segments[3]
                )
            }
        },
    }
}

/// The account a password was presented for, in the form the database
/// compares it in: `account.username` is `COLLATE NOCASE`, which folds ASCII
/// letters only, so `Alice`, `ALICE` and `alice` share one window.
pub fn canonical_account(username: &str) -> String {
    username.to_ascii_lowercase()
}

/// The client a request came from (RFC-017 decision 2).
///
/// The peer, unless the peer is a trusted proxy. Then `X-Forwarded-For` —
/// every such header, joined in the order received — is walked from the
/// right: trusted proxies are skipped and the first other address is the
/// client. An entry that does not parse stops the walk at the last trusted hop
/// reached, and a chain of nothing but trusted proxies resolves to the peer:
/// neither ever yields an address the client wrote.
///
/// An IPv4-mapped IPv6 address is read as the IPv4 address it carries, the
/// peer included: a dual-stack listener reports an IPv4 proxy as
/// `::ffff:10.0.0.2`, which an IPv4 network in the list would not contain.
/// The list is compared family by family, so a mapped entry has to have been
/// read as IPv4 too — [`trusted_network`] does that when the list is loaded.
pub fn client_address(
    peer: Option<IpAddr>,
    headers: &HeaderMap,
    trusted: &[IpNet],
) -> Option<IpAddr> {
    let peer = unmapped(peer?);
    let is_trusted = |address: &IpAddr| trusted.iter().any(|net| net.contains(address));
    if !is_trusted(&peer) {
        return Some(peer);
    }
    // Split from the raw bytes, entry by entry: a value is never discarded
    // whole because the part a client wrote holds a byte that is not text, or
    // the walk would skip past the entry the proxy appended.
    let entries: Vec<&[u8]> = headers
        .get_all("x-forwarded-for")
        .iter()
        .flat_map(|value| value.as_bytes().split(|byte| *byte == b','))
        .collect();
    let mut last_trusted = peer;
    for entry in entries.iter().rev() {
        let Some(address) = std::str::from_utf8(entry)
            .ok()
            .and_then(|entry| entry.trim().parse::<IpAddr>().ok())
            .map(unmapped)
        else {
            return Some(last_trusted);
        };
        if is_trusted(&address) {
            last_trusted = address;
        } else {
            return Some(address);
        }
    }
    Some(peer)
}

/// A trusted network as [`client_address`] compares it: an IPv4-mapped block,
/// `::ffff:a.b.c.d/(96+n)`, becomes the IPv4 block `a.b.c.d/n` it spells.
///
/// Only a block inside the mapped range is rewritten. A wider IPv6 block, `::/0`
/// say, stays IPv6 and holds no IPv4 client: trusting every IPv6 proxy is not
/// a reason to trust every IPv4 one.
pub fn trusted_network(net: IpNet) -> IpNet {
    match net {
        IpNet::V6(v6) if v6.prefix_len() >= 96 => v6
            .network()
            .to_ipv4_mapped()
            .and_then(|v4| ipnet::Ipv4Net::new(v4, v6.prefix_len() - 96).ok())
            .map_or(net, IpNet::V4),
        _ => net,
    }
}

/// An IPv4-mapped IPv6 address as the IPv4 address it carries; any other
/// address as it is.
fn unmapped(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(address, IpAddr::V4),
        IpAddr::V4(_) => address,
    }
}

/// The bucket a request is counted under: its peer, from axum's
/// `ConnectInfo`, read through [`client_address`]. A request served without
/// `ConnectInfo` — only ever in-process — counts as `unknown`.
pub fn request_bucket(
    extensions: &axum::http::Extensions,
    headers: &HeaderMap,
    trusted: &[IpNet],
) -> String {
    let peer = extensions
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|info| info.0.ip());
    address_bucket(client_address(peer, headers, trusted))
}

/// The limiter, the gate and the tally. One per process, shared by the three
/// surfaces.
pub struct SignInGuard {
    table: Mutex<Table>,
    gate: Gate,
    tally: Tally,
}

#[derive(Default)]
struct Table {
    windows: HashMap<[u8; 32], Window>,
    /// Live account windows per address, the overflow window not counted.
    accounts: HashMap<String, usize>,
    last_prune: Option<Instant>,
}

struct Window {
    address: String,
    overflow: bool,
    attempts: VecDeque<Instant>,
}

impl SignInGuard {
    /// `permits` verifications may run at once; four times as many may wait.
    pub fn new(permits: usize) -> Arc<Self> {
        let permits = permits.max(1);
        Arc::new(Self {
            table: Mutex::new(Table::default()),
            gate: Gate {
                permits: Arc::new(Semaphore::new(permits)),
                waiting: AtomicUsize::new(0),
                max_waiting: permits * WAITING_PER_PERMIT,
            },
            tally: Tally::default(),
        })
    }

    /// The default: `max(2, available CPUs)` verifications at once.
    pub fn with_default_permits() -> Arc<Self> {
        let cpus = std::thread::available_parallelism().map_or(2, usize::from);
        Self::new(cpus.max(2))
    }

    /// Takes a slot for one attempt by `account` from `address`, before
    /// anything is checked. The slot is kept unless the attempt succeeds or
    /// fails for an outage first ([`Reservation::release`]): dropping the
    /// reservation counts it as a failure, which is what an abandoned attempt
    /// is.
    pub fn reserve(
        self: &Arc<Self>,
        address: &str,
        account: &str,
    ) -> Result<Reservation, Throttled> {
        let now = Instant::now();
        let mut table = self
            .table
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if table
            .last_prune
            .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(1))
        {
            table.prune(now);
        }
        let mut key = window_key(address, account);
        let mut overflow = false;
        if !table.windows.contains_key(&key) {
            let accounts = table.accounts.get(address).copied().unwrap_or(0);
            if accounts >= WINDOWS_PER_ADDRESS - 1 {
                key = window_key(address, OVERFLOW_ACCOUNT);
                overflow = true;
            }
        }
        if let Some(window) = table.windows.get_mut(&key) {
            expire(&mut window.attempts, now);
            if window.attempts.len() >= FAILURES_PER_WINDOW {
                return Err(Throttled);
            }
            window.attempts.push_back(now);
        } else {
            if table.windows.len() >= MAX_WINDOWS {
                return Err(Throttled);
            }
            table.windows.insert(
                key,
                Window {
                    address: address.to_owned(),
                    overflow,
                    attempts: VecDeque::from([now]),
                },
            );
            if !overflow {
                *table.accounts.entry(address.to_owned()).or_insert(0) += 1;
            }
        }
        Ok(Reservation {
            guard: Arc::clone(self),
            key,
            at: now,
        })
    }

    /// Waits for a verification permit: at once if one is free, up to five
    /// seconds if the queue has room, `None` at once if it has none.
    pub async fn admit(&self) -> Option<OwnedSemaphorePermit> {
        self.gate.admit().await
    }

    /// Counts a refusal, and logs the first throttle, or the first unreadable
    /// hash, after a quiet minute.
    pub fn note(&self, surface: Surface, refusal: Refusal) {
        self.tally.note(surface, refusal);
    }

    /// Logs the counts once a minute, when any moved.
    pub fn spawn_summary(self: &Arc<Self>) {
        let guard = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(WINDOW);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                guard.tally.summarise();
            }
        });
    }

    /// How many windows the table holds: the test suite's view of the table.
    #[doc(hidden)]
    pub fn window_count(&self) -> usize {
        self.table
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .windows
            .len()
    }

    /// How many verifications are waiting for a permit.
    #[doc(hidden)]
    pub fn waiting(&self) -> usize {
        self.gate.waiting.load(Ordering::SeqCst)
    }
}

/// One reserved attempt. Dropped, it stays counted.
pub struct Reservation {
    guard: Arc<SignInGuard>,
    key: [u8; 32],
    at: Instant,
}

impl Reservation {
    /// Gives the slot back: the attempt succeeded, or failed for an outage
    /// before the password was compared.
    pub fn release(self) {
        let mut table = self
            .guard
            .table
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(window) = table.windows.get_mut(&self.key) {
            if let Some(position) = window.attempts.iter().rposition(|at| *at == self.at) {
                window.attempts.remove(position);
            }
        }
    }
}

impl Table {
    /// Drops expired attempts, then the windows left empty.
    fn prune(&mut self, now: Instant) {
        self.last_prune = Some(now);
        let mut emptied = Vec::new();
        self.windows.retain(|_, window| {
            expire(&mut window.attempts, now);
            let live = !window.attempts.is_empty();
            if !live && !window.overflow {
                emptied.push(window.address.clone());
            }
            live
        });
        for address in emptied {
            if let Some(count) = self.accounts.get_mut(&address) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.accounts.remove(&address);
                }
            }
        }
    }
}

fn expire(attempts: &mut VecDeque<Instant>, now: Instant) {
    while attempts
        .front()
        .is_some_and(|at| now.duration_since(*at) >= WINDOW)
    {
        attempts.pop_front();
    }
}

/// Hashed, so the table never holds a name in clear.
fn window_key(address: &str, account: &str) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(address.as_bytes());
    digest.update([0]);
    digest.update(account.as_bytes());
    digest.finalize().into()
}

struct Gate {
    permits: Arc<Semaphore>,
    waiting: AtomicUsize,
    max_waiting: usize,
}

impl Gate {
    async fn admit(&self) -> Option<OwnedSemaphorePermit> {
        if let Ok(permit) = Arc::clone(&self.permits).try_acquire_owned() {
            return Some(permit);
        }
        if self.waiting.fetch_add(1, Ordering::SeqCst) >= self.max_waiting {
            self.waiting.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        // Counted out however the wait ends, a dropped request included.
        let _waiting = Waiting(&self.waiting);
        tokio::time::timeout(PERMIT_WAIT, Arc::clone(&self.permits).acquire_owned())
            .await
            .ok()?
            .ok()
    }
}

struct Waiting<'a>(&'a AtomicUsize);

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct Tally {
    /// By surface, then by refusal, in declaration order.
    counts: [[AtomicU64; 4]; 3],
    /// When each refusal last had a line of its own, by refusal.
    last_logged: Mutex<[Option<Instant>; 4]>,
}

impl Tally {
    fn note(&self, surface: Surface, refusal: Refusal) {
        self.counts[surface as usize][refusal as usize].fetch_add(1, Ordering::Relaxed);
        let message = match refusal {
            Refusal::Throttled => "sign-in attempts are being throttled",
            Refusal::UnreadableHash => "a stored password hash cannot be read",
            Refusal::WrongCredentials | Refusal::Busy => return,
        };
        let now = Instant::now();
        let mut last = self
            .last_logged
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let last = &mut last[refusal as usize];
        if last.is_none_or(|at| now.duration_since(at) >= WINDOW) {
            *last = Some(now);
            tracing::warn!(surface = surface.as_str(), "{message}");
        }
    }

    fn summarise(&self) {
        let take = |surface: Surface, refusal: Refusal| {
            self.counts[surface as usize][refusal as usize].swap(0, Ordering::Relaxed)
        };
        let mut lines = Vec::new();
        for surface in [Surface::Native, Surface::Web, Surface::Subsonic] {
            let throttled = take(surface, Refusal::Throttled);
            let wrong = take(surface, Refusal::WrongCredentials);
            let busy = take(surface, Refusal::Busy);
            let unreadable = take(surface, Refusal::UnreadableHash);
            if throttled + wrong + busy + unreadable > 0 {
                lines.push((surface, throttled, wrong, busy, unreadable));
            }
        }
        for (surface, throttled, wrong_credentials, busy, unreadable_hashes) in lines {
            tracing::warn!(
                surface = surface.as_str(),
                throttled,
                wrong_credentials,
                busy,
                unreadable_hashes,
                "sign-in refusals in the last minute"
            );
        }
    }
}

impl Surface {
    fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Web => "web",
            Self::Subsonic => "subsonic",
        }
    }
}

#[cfg(test)]
mod tests;
