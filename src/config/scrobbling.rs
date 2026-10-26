//! Scrobbling as the deployment sees it (RFC-010): how the queue drains, the
//! destinations the operator declared, and the Last.fm application.

use std::time::Duration;

/// How the server drains what it owes a third party. RFC-010.
///
/// Apart from the credentials, which belong to an account and are posed through
/// the API — decision 9. What is left here is what belongs to the deployment:
/// how often the queue moves, how many times a failure is worth repeating, and
/// how long a queue may sit still before the operator should be told it has
/// stopped.
#[derive(Debug, Clone, Copy)]
pub struct ScrobbleLimits {
    /// How often the drain walks the queue.
    pub drain_interval: Duration,
    /// How long one submission may take before the drain stops waiting for it.
    ///
    /// A deployment setting rather than a constant because decision 9 says so,
    /// and it is load-bearing: the drain is a background task, so a destination
    /// that accepts a connection and then never answers would hold the queue
    /// still for the life of the process — the silent failure `degraded` exists
    /// to surface, arriving by the one route that would also stop `degraded`
    /// from ever being computed.
    pub request_timeout: Duration,
    /// How many times one listen may be submitted before it is abandoned and
    /// counted. Bounded because a queue that never empties is a fault and not a
    /// state — decision 6.
    pub max_attempts: u32,
    /// How long the oldest waiting listen may be waiting before the link is
    /// reported `degraded`. A valid token with a queue that has not moved for
    /// hours is the silent failure a durable queue exists to make visible —
    /// decision 12.
    pub stale_after: Duration,
    /// How many listens one drain pass takes. It bounds the pass, never the
    /// request: decision 11 sends one listen per request whatever this says.
    pub batch: usize,
    /// How long a finished entry stays readable before the purge takes it.
    ///
    /// `sent`, `rejected`, `abandoned`, `cancelled` and `discarded` only.
    /// `uncertain` is kept for as long as nobody has answered it, whatever this
    /// says: taking away an entry that is still asking a person would be
    /// deciding in their place, which is the one thing decision 13 refuses.
    ///
    /// No floor, unlike the library event feed. There, cutting the head off a
    /// quiet library sends a device back to the snapshot; here nobody resumes a
    /// cursor, and what a link publishes is read from `pending` and `uncertain`
    /// rows that no purge touches.
    pub retention_days: u32,
}

/// How many times a listen is offered before the queue gives up on it.
///
/// Thirty, because the waits grow to an hour and stop there: six doublings from
/// a minute, then an hour apiece. Thirty submissions therefore span a little
/// over **a day** of a destination being down, which is the span worth
/// surviving — a nightly maintenance window, a regional outage, a certificate
/// nobody renewed until morning.
///
/// It was eight, with a comment claiming the same day; eight delivers two hours
/// and three minutes. The comment was the honest statement of what this is for,
/// so the number moved to meet it rather than the sentence being trimmed to fit.
/// Giving up early buys nothing here: abandoning a listen is a permanent hole in
/// someone's history, and unlike a retry after an ambiguous answer it risks no
/// duplicate at all. `the_default_attempt_cap_carries_a_listen_across_a_day_of_outage`
/// keeps this paragraph and the arithmetic from drifting apart again.
pub const DEFAULT_SCROBBLE_MAX_ATTEMPTS: u32 = 30;

/// The name an instance takes when the operator declared only an address.
///
/// It is written into the link like any other name, so the shorthand and the
/// long form produce the same rows: `WAVEFLOW_SCROBBLE_MALOJA_URL=https://…`
/// and `…=default=https://…` are one configuration, and an operator who later
/// adds a second instance does not invalidate the links the first one holds.
pub const DEFAULT_SCROBBLE_DESTINATION: &str = "default";

/// The Last.fm application an operator registered for this server.
///
/// `WAVEFLOW_SCROBBLE_LASTFM_API_KEY` and `WAVEFLOW_SCROBBLE_LASTFM_SECRET`,
/// both or neither. Without them Last.fm is simply unavailable: RFC-010
/// decision 4 forbids shipping any provider credential in an AGPL binary, so
/// these belong to the deployment and to nobody else.
///
/// The operator also registers, at Last.fm, the callback URL their
/// `WAVEFLOW_PUBLIC_URL` produces. Their documentation does not say whether the
/// `cb` parameter is checked against the registered domain, and betting on a
/// tolerance nobody promised is not a deployment step — nothing on this side
/// can verify it either way.
#[derive(Clone)]
pub struct LastFmApplication {
    pub api_key: String,
    /// Signs every call. Written by hand below rather than derived, because a
    /// `Config` is printed at startup under `RUST_LOG=debug` and this field is
    /// the one thing in it that must never appear there.
    pub secret: String,
}

impl std::fmt::Debug for LastFmApplication {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LastFmApplication")
            .field("api_key", &self.api_key)
            .field("secret", &"[redacted]")
            .finish()
    }
}

/// One instance of one destination, as the operator declared it.
#[derive(Debug, Clone)]
pub struct ScrobbleDestination {
    pub provider: crate::services::ScrobbleProvider,
    /// What a member names in a path. Validated against the path alphabet at
    /// boot — see `scrobblers::validate_destination_name`.
    pub name: String,
    /// Where it lives. Validated by `scrobblers::validate_destination`, so it
    /// carries no credentials, no query and no fragment.
    pub url: url::Url,
    /// What a link keeps so it can tell this is still the same machine, even
    /// when the name has not changed. See
    /// `scrobblers::destination_fingerprint`.
    pub fingerprint: String,
}

/// The instance `Config::for_data_dir` declares. See the comment there.
pub(super) fn test_destinations() -> Vec<ScrobbleDestination> {
    // `expect` rather than a fallible signature: the literal is right here, and
    // a `Config` builder that could fail would push the failure into every
    // test's first line for no gain.
    // All three recipients, because an account cannot link a name nobody
    // declared, and more than one test drives a queue holding two of them —
    // a destination this process cannot reach must not hold up one it can.
    //
    // Declaring a destination is not registering an adapter, and the difference
    // is what those tests stand on: a Last.fm instance with no application
    // credentials configured is declared and adapterless, which is a shape a
    // real deployment has too.
    [
        (
            crate::services::ScrobbleProvider::ListenBrainz,
            "http://127.0.0.1:1",
        ),
        (
            crate::services::ScrobbleProvider::Maloja,
            "http://127.0.0.1:2",
        ),
        (
            crate::services::ScrobbleProvider::LastFm,
            "http://127.0.0.1:3",
        ),
    ]
    .into_iter()
    .map(|(provider, base)| {
        let url = crate::remote::scrobblers::validate_destination(base, true)
            .expect("a loopback destination nothing is listening on");
        let fingerprint = crate::remote::scrobblers::destination_fingerprint(&url);
        ScrobbleDestination {
            provider,
            name: DEFAULT_SCROBBLE_DESTINATION.to_owned(),
            url,
            fingerprint,
        }
    })
    .collect()
}

/// Reads one recipient's declared instances.
///
/// Either a bare URL — one instance, named [`DEFAULT_SCROBBLE_DESTINATION`] —
/// or a comma-separated list of `name=url`. The two forms produce the same
/// rows, so adding a second instance later does not rename the first.
///
/// **Everything here is refused at boot rather than at the first submission.**
/// A server that starts with scrobbling quietly switched off by a typo is
/// exactly the silent failure RFC-010 spends itself making visible. An operator
/// who wants no adapter for a recipient says so by setting the variable empty,
/// which is a different thing from misspelling it.
pub(super) fn parse_destinations(
    provider: crate::services::ScrobbleProvider,
    variable: &str,
    configured: &str,
    allow_plaintext: bool,
) -> anyhow::Result<Vec<ScrobbleDestination>> {
    let configured = configured.trim();
    if configured.is_empty() {
        return Ok(Vec::new());
    }
    let mut destinations: Vec<ScrobbleDestination> = Vec::new();
    for entry in configured
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        // Split on the first `=` — but only when what precedes it could be a
        // name at all.
        //
        // **The obvious version of this leaks a password.** It said the first
        // `=` cannot occur inside a URL because a scheme has none, which is
        // true of the scheme and false of everything after it:
        // `https://wf:pa=ss@host` splits into `https://wf:pa` and `ss@host`,
        // and the first half then fails the name check and is quoted back in
        // the error — scheme, account and half the password, in a startup log.
        // The paragraph below promises the URL never reaches a message, and
        // this was the path that broke it.
        //
        // A name is letters, digits, `-`, `_` and `.`. Neither `:` nor `/` can
        // appear in one, so their presence before the `=` means the separator
        // was inside a URL, and the whole entry is read as a bare address. A
        // mistyped *name* — `al ice=…` — has neither, so it is still reported
        // by name, which is what an operator needs to see.
        let named = entry
            .split_once('=')
            .filter(|(before, _)| !before.contains([':', '/']));
        let (name, raw) = match named {
            Some((name, raw)) => (name.trim(), raw.trim()),
            None => (DEFAULT_SCROBBLE_DESTINATION, entry),
        };
        crate::remote::scrobblers::validate_destination_name(name)
            .map_err(|error| anyhow::anyhow!("invalid {variable} entry {name:?}: {error}"))?;
        // **Never the URL itself in the message.** `validate_destination`
        // refuses a destination carrying credentials, so the one thing this
        // branch is most likely to be handed is the one thing that must not be
        // printed: `http://user:secret@host` would land in a startup log, or a
        // support paste, in full. The name and the variant say enough.
        let url = crate::remote::scrobblers::validate_destination(raw, allow_plaintext)
            .map_err(|error| anyhow::anyhow!("invalid {variable} entry {name:?}: {error}"))?;
        if destinations.iter().any(|existing| existing.name == name) {
            anyhow::bail!("{variable} declares {name:?} twice");
        }
        // Two names for one machine would give a member two ways to reach the
        // same profile and two links to keep in step, and the second would look
        // like a different destination in every counter.
        let fingerprint = crate::remote::scrobblers::destination_fingerprint(&url);
        if let Some(twin) = destinations
            .iter()
            .find(|existing| existing.fingerprint == fingerprint)
        {
            anyhow::bail!(
                "{variable} declares {name:?} and {:?} at the same address",
                twin.name
            );
        }
        destinations.push(ScrobbleDestination {
            provider,
            name: name.to_owned(),
            url,
            fingerprint,
        });
    }
    Ok(destinations)
}
