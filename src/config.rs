//! WaveFlow v2 process configuration.
//!
//! Environment access is centralised here so domain and repository code stay
//! deterministic and testable.

use std::{net::SocketAddr, path::PathBuf, time::Duration};

/// What the server accepts from a client that offers it a file.
///
/// Every one of these is a bound on someone else's disk, which is why they sit
/// together rather than scattered among the tunables: receiving a file is the
/// only thing the server does that cannot be undone by restarting it.
#[derive(Debug, Clone, Copy)]
pub struct UploadLimits {
    /// The largest single file the server will take.
    pub max_file_bytes: i64,
    /// How much of a library's disk received files may occupy in total, open
    /// sessions included — a session reserves what it declared, or two
    /// negotiations racing would each be told there was room for both.
    pub library_quota_bytes: i64,
    /// The size a client should send each fragment at. Advertised by the
    /// negotiation rather than assumed, so it can move without a client
    /// release.
    pub chunk_bytes: i64,
    /// How many offers one negotiation may carry. Bounded because the batch is
    /// the answer to five thousand round trips, and an unbounded array would
    /// trade them for one unbounded body.
    pub batch_limit: usize,
    /// How many sessions one account may hold open at once, across libraries —
    /// the same shape as the per-user transcode limit, and for the same reason:
    /// a client with five thousand files must not open five thousand of
    /// anything.
    pub sessions_per_user: usize,
    /// How long an untouched session survives. Generous, because a large file
    /// on a domestic link is measured in hours, not minutes.
    pub session_ttl: Duration,
}

/// How long a library's change feed keeps what it has written.
///
/// The floor wins over the age: a library below it keeps everything, however
/// old. RFC-007 decision 7.
#[derive(Debug, Clone, Copy)]
pub struct LibraryEventRetention {
    /// Whole days. Only what is **strictly** older is cut, so an event exactly
    /// this old survives — the same exclusive bound `stream_ticket::verify`
    /// uses, and the cautious direction: keeping one event too many breaks
    /// nobody, cutting one too many sends somebody back to the snapshot.
    pub days: u32,
    /// The fewest events a library keeps whatever their age.
    pub min_events: i64,
}

/// What the server accepts when a member attaches a loop to a track.
///
/// Apart from [`UploadLimits`] rather than folded into it: the two magazines
/// can live on different disks, and mixing the quotas would let loops starve
/// the space the upload quota exists to protect — the music.
#[derive(Debug, Clone, Copy)]
pub struct CanvasLimits {
    /// The largest canvas the server will take. The route derives its own body
    /// ceiling from this, so it is also the largest body that route accepts.
    pub max_bytes: i64,
    /// How long a loop may run. Not prudence: without it, "a short loop"
    /// becomes video hosting, which is a different product with different
    /// costs.
    pub max_duration_secs: u32,
    /// How much of a library's disk canvases may occupy. Counted in distinct
    /// blobs a library references, never in links, so an album's shared loop is
    /// billed once however many tracks name it.
    pub library_quota_bytes: i64,
}

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

#[derive(Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub public_url: Option<String>,
    pub request_timeout: Duration,
    pub data_dir: PathBuf,
    pub database_path: PathBuf,
    pub instance_key_path: PathBuf,
    pub artwork_dir: PathBuf,
    pub db_max_connections: u32,
    pub sqlite_busy_timeout: Duration,
    pub access_token_ttl: Duration,
    /// Lifetime of a browser stream ticket. It must outlive a full listen, not
    /// just the initial request: the browser reuses the same URL for every
    /// range request, so a seek late in a long track still redeems the original
    /// ticket. Access is re-checked on every redemption, so this bounds how
    /// long a leaked URL stays useful, not how long access itself lasts.
    pub stream_ticket_ttl: Duration,
    pub refresh_token_ttl: Duration,
    pub scan_interval: Option<Duration>,
    pub scan_parallelism: usize,
    pub ffmpeg_path: PathBuf,
    pub ffprobe_path: PathBuf,
    pub transcode_cache_dir: PathBuf,
    pub transcode_cache_max_bytes: u64,
    pub transcode_global_limit: usize,
    pub transcode_per_user_limit: usize,
    /// What the server will accept when a library opts in to receiving files.
    ///
    /// `WAVEFLOW_UPLOAD_MAX_FILE_BYTES`, `WAVEFLOW_UPLOAD_LIBRARY_QUOTA_BYTES`,
    /// `WAVEFLOW_UPLOAD_CHUNK_BYTES`, `WAVEFLOW_UPLOAD_BATCH_LIMIT`,
    /// `WAVEFLOW_UPLOAD_SESSIONS_PER_USER`,
    /// `WAVEFLOW_UPLOAD_SESSION_TTL_SECS`.
    ///
    /// None of these matter until an operator sets `accepts_uploads` on a
    /// library: a server that has only been upgraded accepts nothing.
    pub uploads: UploadLimits,
    /// Where canvas blobs live: a content-addressed store beside `artwork_dir`,
    /// under `data/`.
    ///
    /// Derived from `WAVEFLOW_DATA_DIR` rather than set on its own, exactly as
    /// `artwork_dir` is. Not the library root — that is the operator's
    /// collection, and an object the server produced has no business changing
    /// what "delete the library" means. Not `artwork_dir` either: the `artwork`
    /// table constrains the format to a set of images and `read_artwork` holds
    /// the matching MIME map, so a video in that directory would oblige the two
    /// lists to agree forever.
    pub canvas_dir: PathBuf,
    /// How long a library's change feed keeps what it has written.
    ///
    /// `WAVEFLOW_LIBRARY_EVENT_RETENTION_DAYS`,
    /// `WAVEFLOW_LIBRARY_EVENT_RETENTION_MIN`.
    ///
    /// Two bounds because either alone fails in the opposite direction: an age
    /// alone lets a library that rescans daily grow without limit, and a count
    /// alone cuts the head off a quiet one whose ten thousand events cover two
    /// years. RFC-007 decision 7.
    pub library_event_retention: LibraryEventRetention,
    /// What the server accepts when a member attaches a loop to a track.
    ///
    /// `WAVEFLOW_CANVAS_MAX_BYTES`, `WAVEFLOW_CANVAS_MAX_DURATION_SECS`,
    /// `WAVEFLOW_CANVAS_LIBRARY_QUOTA_BYTES`.
    ///
    /// Gated by `accepts_canvas`, which is its own flag rather than the upload
    /// one. A read-only server that refuses to grow in audio may still want a
    /// few hundred kilobytes of loop, and that is the most common installation
    /// there is — sharing the flag made this whole feature inert exactly where
    /// it is most wanted.
    pub canvas: CanvasLimits,
    /// How the server drains what it owes a third party.
    ///
    /// `WAVEFLOW_SCROBBLE_DRAIN_INTERVAL_SECS`,
    /// `WAVEFLOW_SCROBBLE_REQUEST_TIMEOUT_SECS`,
    /// `WAVEFLOW_SCROBBLE_MAX_ATTEMPTS`, `WAVEFLOW_SCROBBLE_STALE_AFTER_SECS`,
    /// `WAVEFLOW_SCROBBLE_BATCH`, `WAVEFLOW_SCROBBLE_RETENTION_DAYS`.
    ///
    /// None of these matter until an account links a destination: a server that
    /// has only been upgraded makes no outbound request at all.
    pub scrobbling: ScrobbleLimits,
    /// Every instance of every destination this server knows, validated at
    /// startup.
    ///
    /// `WAVEFLOW_SCROBBLE_LISTENBRAINZ_URL`, `WAVEFLOW_SCROBBLE_MALOJA_URL` and
    /// `WAVEFLOW_SCROBBLE_LASTFM_URL`. Each takes either a bare URL — one
    /// instance, named `default` — or a comma-separated list of `name=url`
    /// pairs. Set one empty to register no adapter for that recipient at all.
    ///
    /// A destination is the operator's setting and never an account's —
    /// RFC-010 decision 10 — so it is here rather than on `scrobble_link`. A
    /// member picks a *name* among these; they never describe a URL. It costs
    /// nothing while nobody has linked a token: an adapter with no
    /// authorisation behind it is never handed a listen.
    ///
    /// **Plural because self-hosting is plural.** One field per recipient
    /// assumed a singular Maloja does not have: on a family server everyone has
    /// their own instance, and a single field forces them to share one or go
    /// without.
    ///
    /// The one thing this spelling cannot express is a URL containing a comma,
    /// which would need a separator nothing else in this file uses. A path with
    /// a comma in it is not a shape these three destinations have.
    pub destinations: Vec<ScrobbleDestination>,
    /// Whether an outbound destination may be plain HTTP.
    ///
    /// `WAVEFLOW_SCROBBLE_ALLOW_PLAINTEXT`, off by default. The escape exists
    /// because Maloja and ListenBrainz self-host and a container on the
    /// operator's own network is a reasonable destination; the default refuses
    /// it because the request carries a personal token.
    ///
    /// **On is not a blank cheque.** Even set, plaintext only reaches a host
    /// that looks like it is on that network — see `validate_destination`. The
    /// flag says "this address is mine", not "send my members' tokens in clear
    /// wherever I point you".
    pub outbound_allow_plaintext: bool,
    pub allowed_origins: Vec<axum::http::HeaderValue>,
    /// How the catalogue decides which row a scanned file belongs to.
    ///
    /// `WAVEFLOW_PID_ALBUM`, `WAVEFLOW_PID_TRACK`, `WAVEFLOW_PID_ARTIST`.
    ///
    /// Changing one of these re-identifies every album, artist or track it
    /// governs, so the active values are persisted and compared at boot: an
    /// instance configured differently from the run that built its catalogue
    /// schedules a full rescan rather than serving a catalogue keyed under a
    /// rule it no longer follows.
    pub pid: crate::pid::PidSpecs,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("bind_addr", &self.bind_addr)
            .field("public_url", &self.public_url)
            .field("request_timeout", &self.request_timeout)
            .field("data_dir", &self.data_dir)
            .field("database_path", &self.database_path)
            .field("instance_key_path", &self.instance_key_path)
            .field("artwork_dir", &self.artwork_dir)
            .field("db_max_connections", &self.db_max_connections)
            .field("sqlite_busy_timeout", &self.sqlite_busy_timeout)
            .field("access_token_ttl", &self.access_token_ttl)
            .field("stream_ticket_ttl", &self.stream_ticket_ttl)
            .field("refresh_token_ttl", &self.refresh_token_ttl)
            .field("scan_interval", &self.scan_interval)
            .field("scan_parallelism", &self.scan_parallelism)
            .field("ffmpeg_path", &self.ffmpeg_path)
            .field("ffprobe_path", &self.ffprobe_path)
            .field("transcode_cache_dir", &self.transcode_cache_dir)
            .field("transcode_cache_max_bytes", &self.transcode_cache_max_bytes)
            .field("transcode_global_limit", &self.transcode_global_limit)
            .field("transcode_per_user_limit", &self.transcode_per_user_limit)
            .field("uploads", &self.uploads)
            .field("library_event_retention", &self.library_event_retention)
            .field("canvas_dir", &self.canvas_dir)
            .field("canvas", &self.canvas)
            .field("scrobbling", &self.scrobbling)
            .field("destinations", &self.destinations)
            .field("outbound_allow_plaintext", &self.outbound_allow_plaintext)
            .field("allowed_origins", &self.allowed_origins)
            .field("pid", &self.pid)
            .finish()
    }
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let data_dir = std::env::var_os("WAVEFLOW_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("data"));

        let bind_addr = parse_env("WAVEFLOW_BIND", "127.0.0.1:4533")?;
        let public_url = std::env::var("WAVEFLOW_PUBLIC_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| normalize_public_url(&value))
            .transpose()?;
        let request_timeout_secs = parse_positive_env("WAVEFLOW_REQUEST_TIMEOUT_SECS", 30u64)?;
        let db_max_connections = parse_positive_env("WAVEFLOW_DB_MAX_CONNECTIONS", 8u32)?;
        let sqlite_busy_timeout_ms =
            parse_positive_env("WAVEFLOW_SQLITE_BUSY_TIMEOUT_MS", 5_000u64)?;
        let stream_ticket_ttl_secs =
            parse_positive_env("WAVEFLOW_STREAM_TICKET_TTL_SECS", 60 * 60u64)?;
        let access_token_ttl_secs =
            parse_positive_env("WAVEFLOW_ACCESS_TOKEN_TTL_SECS", 15 * 60u64)?;
        let refresh_token_ttl_secs =
            parse_positive_env("WAVEFLOW_REFRESH_TOKEN_TTL_SECS", 30 * 24 * 60 * 60u64)?;
        let scan_interval_secs = std::env::var("WAVEFLOW_SCAN_INTERVAL_SECS")
            .unwrap_or_else(|_| "21600".to_owned())
            .parse::<u64>()
            .map_err(|error| anyhow::anyhow!("invalid WAVEFLOW_SCAN_INTERVAL_SECS: {error}"))?;
        let scan_parallelism = parse_positive_env(
            "WAVEFLOW_SCAN_PARALLELISM",
            std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(4)
                .clamp(1, 16),
        )?;
        let ffmpeg_path = std::env::var_os("WAVEFLOW_FFMPEG_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("ffmpeg"));
        let ffprobe_path = std::env::var_os("WAVEFLOW_FFPROBE_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("ffprobe"));
        let transcode_cache_max_bytes =
            parse_positive_env("WAVEFLOW_TRANSCODE_CACHE_MAX_BYTES", 10_737_418_240u64)?;
        let transcode_global_limit = parse_positive_env("WAVEFLOW_TRANSCODE_GLOBAL_LIMIT", 4usize)?;
        let transcode_per_user_limit =
            parse_positive_env("WAVEFLOW_TRANSCODE_PER_USER_LIMIT", 2usize)?;
        let uploads = UploadLimits {
            max_file_bytes: parse_positive_env("WAVEFLOW_UPLOAD_MAX_FILE_BYTES", 1_073_741_824i64)?,
            library_quota_bytes: parse_positive_env(
                "WAVEFLOW_UPLOAD_LIBRARY_QUOTA_BYTES",
                53_687_091_200i64,
            )?,
            chunk_bytes: parse_positive_env("WAVEFLOW_UPLOAD_CHUNK_BYTES", 4_194_304i64)?,
            batch_limit: parse_positive_env("WAVEFLOW_UPLOAD_BATCH_LIMIT", 200usize)?,
            sessions_per_user: parse_positive_env("WAVEFLOW_UPLOAD_SESSIONS_PER_USER", 4usize)?,
            session_ttl: Duration::from_secs(parse_positive_env(
                "WAVEFLOW_UPLOAD_SESSION_TTL_SECS",
                86_400u64,
            )?),
        };
        validate_uploads(&uploads)?;
        let canvas = CanvasLimits {
            // A Spotify-style loop is a few seconds of portrait video: a few
            // hundred kilobytes in practice. Four megabytes is comfortably
            // above anything that is still a loop, and low enough that the
            // body ceiling derived from it stays a ceiling.
            max_bytes: parse_positive_env("WAVEFLOW_CANVAS_MAX_BYTES", 4_194_304i64)?,
            // Those loops run three to eight seconds. Fifteen leaves room for
            // an unusual one without leaving room for an episode.
            max_duration_secs: parse_positive_env("WAVEFLOW_CANVAS_MAX_DURATION_SECS", 15u32)?,
            // A gigabyte is thousands of real canvases, and still a number an
            // operator can reason about against a disk.
            library_quota_bytes: parse_positive_env(
                "WAVEFLOW_CANVAS_LIBRARY_QUOTA_BYTES",
                1_073_741_824i64,
            )?,
        };
        validate_canvas(&canvas)?;
        let scrobbling = ScrobbleLimits {
            // A minute is far below what any destination considers a listening
            // history's resolution, and far above what a queue of a few rows
            // costs to walk.
            drain_interval: Duration::from_secs(parse_positive_env(
                "WAVEFLOW_SCROBBLE_DRAIN_INTERVAL_SECS",
                60u64,
            )?),
            max_attempts: parse_positive_env(
                "WAVEFLOW_SCROBBLE_MAX_ATTEMPTS",
                DEFAULT_SCROBBLE_MAX_ATTEMPTS,
            )?,
            stale_after: Duration::from_secs(parse_positive_env(
                "WAVEFLOW_SCROBBLE_STALE_AFTER_SECS",
                3_600u64,
            )?),
            batch: parse_positive_env("WAVEFLOW_SCROBBLE_BATCH", 50usize)?,
            // Thirty days: long enough to explain a failure somebody noticed
            // last week, short enough that the queue does not become a second
            // listening history. A setting, not a carved number — whoever
            // diagnoses a three-month outage lengthens it, exactly as they
            // widen the library event window.
            retention_days: parse_positive_env("WAVEFLOW_SCROBBLE_RETENTION_DAYS", 30u32)?,
            // Thirty seconds is far past any of the three destinations'
            // ordinary latency and far short of a drain that has stopped. It
            // bounds one submission, never the pass: the pass is bounded by
            // `batch`.
            request_timeout: Duration::from_secs(parse_positive_env(
                "WAVEFLOW_SCROBBLE_REQUEST_TIMEOUT_SECS",
                30u64,
            )?),
        };
        // Both refuse zero and negatives at startup rather than falling back:
        // every fallback for a bound is wrong, and the operator is turned away
        // where they can see why. No ceiling — an enormous value means "purge
        // nothing", which is safe and legible.
        let library_event_retention = LibraryEventRetention {
            days: parse_positive_env("WAVEFLOW_LIBRARY_EVENT_RETENTION_DAYS", 30u32)?,
            min_events: parse_positive_env("WAVEFLOW_LIBRARY_EVENT_RETENTION_MIN", 10_000i64)?,
        };
        if transcode_per_user_limit > transcode_global_limit {
            anyhow::bail!(
                "WAVEFLOW_TRANSCODE_PER_USER_LIMIT cannot exceed WAVEFLOW_TRANSCODE_GLOBAL_LIMIT"
            );
        }
        // Refused here rather than at the first submission, for the same reason
        // `WAVEFLOW_PUBLIC_URL` is: booting with scrobbling silently switched
        // off because of a typo is the exact silent failure RFC-010 spends
        // itself making visible. An operator who wants no adapter says so by
        // setting this empty, which is a different thing from misspelling it.
        let outbound_allow_plaintext = parse_bool_env("WAVEFLOW_SCROBBLE_ALLOW_PLAINTEXT", false)?;
        let mut destinations = Vec::new();
        for (provider, variable, default) in [
            (
                crate::services::ScrobbleProvider::ListenBrainz,
                "WAVEFLOW_SCROBBLE_LISTENBRAINZ_URL",
                "https://api.listenbrainz.org",
            ),
            (
                crate::services::ScrobbleProvider::Maloja,
                "WAVEFLOW_SCROBBLE_MALOJA_URL",
                // No default: Maloja is self-hosted by nature, so there is no
                // public instance to point at, and guessing one would be
                // inventing a destination the operator never named.
                "",
            ),
            (
                crate::services::ScrobbleProvider::LastFm,
                "WAVEFLOW_SCROBBLE_LASTFM_URL",
                "https://ws.audioscrobbler.com",
            ),
        ] {
            let configured = std::env::var(variable).unwrap_or_else(|_| default.to_owned());
            destinations.extend(parse_destinations(
                provider,
                variable,
                &configured,
                outbound_allow_plaintext,
            )?);
        }

        let allowed_origins = std::env::var("WAVEFLOW_ALLOWED_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|origin| !origin.is_empty())
            .map(|origin| {
                origin.parse::<axum::http::HeaderValue>().map_err(|error| {
                    anyhow::anyhow!("invalid WAVEFLOW_ALLOWED_ORIGINS entry {origin:?}: {error}")
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        // Parsed here rather than at first use: a spec that cannot be parsed —
        // a recursive `albumid` above all — would otherwise re-identify the
        // whole catalogue silently at the next scan. Refusing to boot says so.
        let pid = crate::pid::PidSpecs {
            album: parse_pid_spec(
                "WAVEFLOW_PID_ALBUM",
                DEFAULT_PID_ALBUM,
                PidSpecKind::MayNotReferenceAlbumId,
            )?,
            track: parse_pid_spec(
                "WAVEFLOW_PID_TRACK",
                DEFAULT_PID_TRACK,
                PidSpecKind::MayReferenceAlbumId,
            )?,
            artist: parse_pid_spec(
                "WAVEFLOW_PID_ARTIST",
                DEFAULT_PID_ARTIST,
                PidSpecKind::MayNotReferenceAlbumId,
            )?,
        };

        if refresh_token_ttl_secs <= access_token_ttl_secs {
            anyhow::bail!(
                "WAVEFLOW_REFRESH_TOKEN_TTL_SECS must be greater than WAVEFLOW_ACCESS_TOKEN_TTL_SECS"
            );
        }
        let transcode_cache_dir = data_dir.join("transcode-cache");
        let canvas_dir = data_dir.join("canvas");

        Ok(Self {
            bind_addr,
            public_url,
            request_timeout: Duration::from_secs(request_timeout_secs),
            database_path: data_dir.join("waveflow.db"),
            instance_key_path: data_dir.join("instance.key"),
            artwork_dir: data_dir.join("artwork"),
            data_dir,
            db_max_connections,
            sqlite_busy_timeout: Duration::from_millis(sqlite_busy_timeout_ms),
            access_token_ttl: Duration::from_secs(access_token_ttl_secs),
            stream_ticket_ttl: Duration::from_secs(stream_ticket_ttl_secs),
            refresh_token_ttl: Duration::from_secs(refresh_token_ttl_secs),
            scan_interval: (scan_interval_secs > 0)
                .then(|| Duration::from_secs(scan_interval_secs)),
            scan_parallelism,
            ffmpeg_path,
            ffprobe_path,
            transcode_cache_dir,
            transcode_cache_max_bytes,
            transcode_global_limit,
            transcode_per_user_limit,
            uploads,
            library_event_retention,
            canvas_dir,
            canvas,
            scrobbling,
            destinations,
            outbound_allow_plaintext,
            allowed_origins,
            pid,
        })
    }

    pub fn for_data_dir(data_dir: PathBuf) -> Self {
        let transcode_cache_dir = data_dir.join("transcode-cache");
        let canvas_dir_for_tests = data_dir.join("canvas");
        Self {
            bind_addr: "127.0.0.1:0".parse().expect("literal socket address"),
            public_url: Some("http://waveflow.test".to_owned()),
            request_timeout: Duration::from_secs(30),
            database_path: data_dir.join("waveflow.db"),
            instance_key_path: data_dir.join("instance.key"),
            artwork_dir: data_dir.join("artwork"),
            data_dir,
            db_max_connections: 4,
            sqlite_busy_timeout: Duration::from_secs(5),
            access_token_ttl: Duration::from_secs(15 * 60),
            stream_ticket_ttl: Duration::from_secs(60 * 60),
            refresh_token_ttl: Duration::from_secs(30 * 24 * 60 * 60),
            scan_interval: None,
            scan_parallelism: 2,
            ffmpeg_path: PathBuf::from("ffmpeg"),
            ffprobe_path: PathBuf::from("ffprobe"),
            transcode_cache_dir,
            transcode_cache_max_bytes: 128 * 1024 * 1024,
            transcode_global_limit: 2,
            transcode_per_user_limit: 1,
            // Small enough that a test can reach every bound without writing a
            // gigabyte, and shaped like production rather than unlimited: a
            // suite that never meets a limit never proves one exists.
            uploads: UploadLimits {
                max_file_bytes: 1024 * 1024,
                library_quota_bytes: 4 * 1024 * 1024,
                chunk_bytes: 64 * 1024,
                batch_limit: 8,
                sessions_per_user: 2,
                session_ttl: Duration::from_secs(3600),
            },
            // Small enough that a test can write past the floor without
            // writing ten thousand rows, and shaped like production rather
            // than unlimited.
            library_event_retention: LibraryEventRetention {
                days: 30,
                min_events: 4,
            },
            canvas_dir: canvas_dir_for_tests,
            // Same reasoning as the upload limits above: small enough that a
            // test can reach every bound, shaped like production rather than
            // unlimited.
            canvas: CanvasLimits {
                max_bytes: 256 * 1024,
                max_duration_secs: 15,
                library_quota_bytes: 1024 * 1024,
            },
            // Same reasoning again: small enough that a test can exhaust the
            // attempts without waiting out eight doublings, shaped like
            // production rather than unlimited. The interval is never reached —
            // `spawn_scrobble_drain` is started by `main`, and tests run the
            // pass themselves.
            scrobbling: ScrobbleLimits {
                drain_interval: Duration::from_secs(60),
                // Long enough that no test double ever meets it by accident;
                // the one test that means to meet it shortens this first.
                request_timeout: Duration::from_secs(30),
                max_attempts: 3,
                stale_after: Duration::from_secs(3600),
                batch: 8,
                retention_days: 30,
            },
            // One instance, declared and unreachable.
            //
            // **Declared**, because a member may only link a name the operator
            // published, and a suite with an empty list could link nothing and
            // would exercise none of the queue.
            //
            // **Unreachable**, because the suite must never reach the real
            // ListenBrainz: port 1 on loopback is a port nothing binds. Every
            // test that drives the queue replaces the adapter with a double
            // through `register_scrobble_target`, and the handful that mean to
            // exercise a real request point this at a server they started
            // themselves — over plain HTTP on loopback, which is why the
            // plaintext escape is on here and off in production.
            destinations: test_destinations(),
            outbound_allow_plaintext: true,
            allowed_origins: Vec::new(),
            // The real defaults, so the whole test suite exercises the specs
            // production runs under rather than a simplified stand-in.
            pid: default_pid_specs(),
        }
    }
}

/// The upload limits that only make sense against each other.
///
/// Apart here rather than inline so the rules can be exercised without
/// rewriting the process environment, which is global and shared by every test
/// running beside it.
fn validate_uploads(uploads: &UploadLimits) -> anyhow::Result<()> {
    if uploads.max_file_bytes > uploads.library_quota_bytes {
        anyhow::bail!(
            "WAVEFLOW_UPLOAD_MAX_FILE_BYTES cannot exceed WAVEFLOW_UPLOAD_LIBRARY_QUOTA_BYTES"
        );
    }
    // The fragment route turns this into a body ceiling, and a value it cannot
    // represent would have to fall back to something. Every fallback for a
    // ceiling is wrong — too small breaks the configured size, too large is no
    // ceiling at all — so it is refused here, where the operator can see why.
    if usize::try_from(uploads.chunk_bytes).is_err() {
        anyhow::bail!("invalid WAVEFLOW_UPLOAD_CHUNK_BYTES: too large for this platform");
    }
    if uploads.chunk_bytes > uploads.max_file_bytes {
        anyhow::bail!("WAVEFLOW_UPLOAD_CHUNK_BYTES cannot exceed WAVEFLOW_UPLOAD_MAX_FILE_BYTES");
    }
    Ok(())
}

/// The canvas limits that only make sense against each other.
fn validate_canvas(canvas: &CanvasLimits) -> anyhow::Result<()> {
    if canvas.max_bytes > canvas.library_quota_bytes {
        anyhow::bail!(
            "WAVEFLOW_CANVAS_MAX_BYTES cannot exceed WAVEFLOW_CANVAS_LIBRARY_QUOTA_BYTES"
        );
    }
    // The route turns this into a body ceiling, and a value it cannot represent
    // would have to fall back to something. Every fallback for a ceiling is
    // wrong, so it is refused here where the operator can see why — the same
    // rule the upload chunk size follows.
    if usize::try_from(canvas.max_bytes).is_err() {
        anyhow::bail!("invalid WAVEFLOW_CANVAS_MAX_BYTES: too large for this platform");
    }
    Ok(())
}

/// The instance `Config::for_data_dir` declares. See the comment there.
fn test_destinations() -> Vec<ScrobbleDestination> {
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
        let url = crate::scrobblers::validate_destination(base, true)
            .expect("a loopback destination nothing is listening on");
        let fingerprint = crate::scrobblers::destination_fingerprint(&url);
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
fn parse_destinations(
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
        // Split on the first `=`, which cannot occur in a scheme: `https://…`
        // has none before the colon, so a bare URL is never mistaken for a
        // named one. A name may not contain `=` either — the alphabet refuses
        // it — so the first separator is the only one.
        let (name, raw) = match entry.split_once('=') {
            Some((name, raw)) => (name.trim(), raw.trim()),
            None => (DEFAULT_SCROBBLE_DESTINATION, entry),
        };
        crate::scrobblers::validate_destination_name(name)
            .map_err(|error| anyhow::anyhow!("invalid {variable} entry {name:?}: {error}"))?;
        // **Never the URL itself in the message.** `validate_destination`
        // refuses a destination carrying credentials, so the one thing this
        // branch is most likely to be handed is the one thing that must not be
        // printed: `http://user:secret@host` would land in a startup log, or a
        // support paste, in full. The name and the variant say enough.
        let url = crate::scrobblers::validate_destination(raw, allow_plaintext)
            .map_err(|error| anyhow::anyhow!("invalid {variable} entry {name:?}: {error}"))?;
        if destinations.iter().any(|existing| existing.name == name) {
            anyhow::bail!("{variable} declares {name:?} twice");
        }
        // Two names for one machine would give a member two ways to reach the
        // same profile and two links to keep in step, and the second would look
        // like a different destination in every counter.
        let fingerprint = crate::scrobblers::destination_fingerprint(&url);
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

fn normalize_public_url(value: &str) -> anyhow::Result<String> {
    let parsed = url::Url::parse(value.trim())
        .map_err(|error| anyhow::anyhow!("invalid WAVEFLOW_PUBLIC_URL: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !matches!(parsed.path(), "" | "/")
    {
        anyhow::bail!(
            "WAVEFLOW_PUBLIC_URL must be an http(s) origin without credentials, path, query or fragment"
        );
    }
    Ok(parsed.origin().ascii_serialization())
}

fn parse_env<T>(name: &str, default: &str) -> anyhow::Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    std::env::var(name)
        .unwrap_or_else(|_| default.to_owned())
        .parse()
        .map_err(|error| anyhow::anyhow!("invalid {name}: {error}"))
}

/// A flag an operator either set on purpose or did not set at all.
///
/// A spelling this does not recognise is refused rather than read as `false`.
/// Every silent fallback in this file is a bug waiting to be filed against
/// something else, and this one especially: `WAVEFLOW_SCROBBLE_ALLOW_PLAINTEXT`
/// guards whether a personal token may cross a network in clear, so an operator
/// who typed `True ` or `on` must be told, not quietly overruled.
fn parse_bool_env(name: &str, default: bool) -> anyhow::Result<bool> {
    let Ok(raw) = std::env::var(name) else {
        return Ok(default);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "" => Ok(default),
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        other => anyhow::bail!("invalid {name}: expected a boolean, found {other:?}"),
    }
}

fn parse_positive_env<T>(name: &str, default: T) -> anyhow::Result<T>
where
    T: std::str::FromStr + PartialOrd + Default + Copy + std::fmt::Display,
    T::Err: std::fmt::Display,
{
    let raw = std::env::var(name).unwrap_or_else(|_| default.to_string());
    let value = raw
        .parse::<T>()
        .map_err(|error| anyhow::anyhow!("invalid {name}: {error}"))?;
    if value <= T::default() {
        anyhow::bail!("invalid {name}: must be greater than zero");
    }
    Ok(value)
}

/// The identity rules an instance runs under unless it says otherwise.
///
/// Taken verbatim from Navidrome, which is the reference these follow: a
/// release identifier when the files carry one, otherwise the album artist,
/// title, version and date together.
const DEFAULT_PID_ALBUM: &str = "musicbrainz_albumid|albumartistid,album,albumversion,releasedate";
const DEFAULT_PID_TRACK: &str = "musicbrainz_trackid|albumid,discnumber,tracknumber,title";
const DEFAULT_PID_ARTIST: &str = "albumartistid";

/// Whether a spec is allowed to name `albumid`.
///
/// The album's own spec is not, and neither is the artist's: both would be
/// asking the album for an answer that depends on themselves.
enum PidSpecKind {
    MayNotReferenceAlbumId,
    MayReferenceAlbumId,
}

impl PidSpecKind {
    fn allows_album_id(&self) -> bool {
        matches!(self, Self::MayReferenceAlbumId)
    }
}

fn parse_pid_spec(
    name: &str,
    default: &str,
    kind: PidSpecKind,
) -> anyhow::Result<crate::pid::PidSpec> {
    let raw = std::env::var(name).unwrap_or_else(|_| default.to_owned());
    crate::pid::PidSpec::parse(&raw, kind.allows_album_id())
        .map_err(|error| anyhow::anyhow!("invalid {name}: {error}"))
}

fn default_pid_spec(default: &str, kind: PidSpecKind) -> crate::pid::PidSpec {
    crate::pid::PidSpec::parse(default, kind.allows_album_id()).expect("built-in pid spec default")
}

fn default_pid_specs() -> crate::pid::PidSpecs {
    crate::pid::PidSpecs {
        album: default_pid_spec(DEFAULT_PID_ALBUM, PidSpecKind::MayNotReferenceAlbumId),
        track: default_pid_spec(DEFAULT_PID_TRACK, PidSpecKind::MayReferenceAlbumId),
        artist: default_pid_spec(DEFAULT_PID_ARTIST, PidSpecKind::MayNotReferenceAlbumId),
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_public_url;
    use super::{parse_destinations, DEFAULT_SCROBBLE_DESTINATION};
    use super::{validate_canvas, CanvasLimits};
    use super::{validate_uploads, UploadLimits};
    use std::time::Duration;

    fn workable() -> UploadLimits {
        UploadLimits {
            max_file_bytes: 1024 * 1024,
            library_quota_bytes: 4 * 1024 * 1024,
            chunk_bytes: 64 * 1024,
            batch_limit: 8,
            sessions_per_user: 2,
            session_ttl: Duration::from_secs(3600),
        }
    }

    #[test]
    fn upload_limits_that_contradict_each_other_are_refused() {
        assert!(validate_uploads(&workable()).is_ok());

        // A fragment bigger than the largest file it could belong to.
        let mut oversized_chunk = workable();
        oversized_chunk.chunk_bytes = oversized_chunk.max_file_bytes + 1;
        assert!(validate_uploads(&oversized_chunk).is_err());

        // A file bigger than the whole library may hold.
        let mut oversized_file = workable();
        oversized_file.max_file_bytes = oversized_file.library_quota_bytes + 1;
        assert!(validate_uploads(&oversized_file).is_err());

        // And a fragment this platform could not turn into a body ceiling. The
        // fallback for that conversion must never be "no ceiling", so the value
        // has to be refused before it reaches one.
        if usize::try_from(i64::MAX).is_err() {
            let mut unrepresentable = workable();
            unrepresentable.chunk_bytes = i64::MAX;
            unrepresentable.max_file_bytes = i64::MAX;
            unrepresentable.library_quota_bytes = i64::MAX;
            assert!(validate_uploads(&unrepresentable).is_err());
        }
    }

    #[test]
    fn canvas_limits_that_contradict_each_other_are_refused() {
        let workable = CanvasLimits {
            max_bytes: 256 * 1024,
            max_duration_secs: 15,
            library_quota_bytes: 1024 * 1024,
        };
        assert!(validate_canvas(&workable).is_ok());

        // A single canvas larger than the whole library may hold: the first one
        // placed would be refused by a quota it can never fit under, which is a
        // misconfiguration rather than a verdict.
        let mut oversized = workable;
        oversized.max_bytes = oversized.library_quota_bytes + 1;
        assert!(validate_canvas(&oversized).is_err());

        // And a ceiling this platform cannot represent, for the same reason the
        // upload chunk size is refused: every fallback for a ceiling is wrong.
        if usize::try_from(i64::MAX).is_err() {
            let unrepresentable = CanvasLimits {
                max_bytes: i64::MAX,
                max_duration_secs: 15,
                library_quota_bytes: i64::MAX,
            };
            assert!(validate_canvas(&unrepresentable).is_err());
        }
    }

    #[test]
    fn public_url_is_reduced_to_a_safe_http_origin() {
        assert_eq!(
            normalize_public_url("https://music.example.com:8443/").unwrap(),
            "https://music.example.com:8443"
        );
        for invalid in [
            "ftp://music.example.com",
            "https://user:secret@music.example.com",
            "https://music.example.com/waveflow",
            "https://music.example.com?token=secret",
        ] {
            assert!(normalize_public_url(invalid).is_err(), "{invalid}");
        }
    }

    /// A destination that cannot be used is never quoted back.
    ///
    /// `validate_destination` refuses a URL carrying credentials, so the value
    /// this error branch is most likely to be holding is precisely the one that
    /// must not be printed — and a startup error goes to a log, a terminal, and
    /// whatever a person pastes into an issue. The repository's rule about
    /// secrets has no exception for an error path.
    ///
    /// It used to live in `tests/scrobbling.rs` against `initialize`, which is
    /// where the guard used to be. Named destinations moved the refusal here,
    /// to the reading of the variable, and the test follows the guard rather
    /// than staying where it was and passing for another reason.
    #[test]
    fn a_refused_destination_is_never_quoted_back_with_its_credentials() {
        // A private host, so the plaintext rule lets this through and the
        // credential rule is what refuses it. That is the path carrying a
        // password.
        let refused = parse_destinations(
            crate::services::ScrobbleProvider::Maloja,
            "WAVEFLOW_SCROBBLE_MALOJA_URL",
            "house=http://wf-user:hunter2@10.0.0.2",
            true,
        )
        .expect_err("a destination carrying credentials must be refused");
        let said = format!("{refused:#}");

        // **Named, never quoted.** An earlier version of this loop put the
        // secret and the whole error into the assertion message, and CodeQL was
        // right to call that a cleartext write of a credential — in the one
        // test whose entire subject is that credentials must not be written.
        // What a failure here needs to say is *which* part came back, not the
        // part itself.
        for (part, secret) in [
            ("the password", "hunter2"),
            ("the account", "wf-user"),
            ("the host", "10.0.0.2"),
        ] {
            assert!(
                !said.contains(secret),
                "the startup error quoted {part} back from the destination"
            );
        }
        // And it still says enough to be fixed by the person who typed it:
        // which variable, which name, and what was wrong with it. Printing
        // `said` is safe here and only here — the loop above has just
        // established that it carries none of the three.
        assert!(
            said.contains("credentials"),
            "the error must name the fault: {said}"
        );
        assert!(
            said.contains("house") && said.contains("WAVEFLOW_SCROBBLE_MALOJA_URL"),
            "the error must name the entry that is wrong: {said}"
        );
    }

    /// Two spellings of one machine are one destination.
    ///
    /// The trailing slash is the one an editor adds without anybody deciding
    /// to, and a fingerprint that moved with it would break every link on a
    /// server at the first restart after a configuration tidy-up.
    #[test]
    fn a_trailing_slash_does_not_move_a_destination() {
        let bare = parse_destinations(
            crate::services::ScrobbleProvider::Maloja,
            "WAVEFLOW_SCROBBLE_MALOJA_URL",
            "https://host/maloja",
            false,
        )
        .expect("a valid destination");
        let slashed = parse_destinations(
            crate::services::ScrobbleProvider::Maloja,
            "WAVEFLOW_SCROBBLE_MALOJA_URL",
            "https://host/maloja/",
            false,
        )
        .expect("a valid destination");
        assert_eq!(bare[0].fingerprint, slashed[0].fingerprint);

        // And the path is *part* of the identity, unlike an origin. Two tenants
        // on one host are two destinations, and giving them one fingerprint
        // would let the identity guard wave through the likeliest move there
        // is.
        let other_tenant = parse_destinations(
            crate::services::ScrobbleProvider::Maloja,
            "WAVEFLOW_SCROBBLE_MALOJA_URL",
            "https://host/maloja-two",
            false,
        )
        .expect("a valid destination");
        assert_ne!(bare[0].fingerprint, other_tenant[0].fingerprint);
    }

    /// A recipient may carry several instances, and neither name nor address
    /// may repeat.
    #[test]
    fn a_recipient_carries_several_instances_each_named_once() {
        let declared = parse_destinations(
            crate::services::ScrobbleProvider::Maloja,
            "WAVEFLOW_SCROBBLE_MALOJA_URL",
            "alice=https://host/alice, bob=https://host/bob",
            false,
        )
        .expect("two instances");
        assert_eq!(declared.len(), 2);
        assert_eq!(declared[0].name, "alice");
        assert_eq!(declared[1].name, "bob");

        // A bare URL is the shorthand for one instance called `default`, so the
        // two spellings produce the same row and adding a second instance later
        // does not rename the first.
        let shorthand = parse_destinations(
            crate::services::ScrobbleProvider::Maloja,
            "WAVEFLOW_SCROBBLE_MALOJA_URL",
            "https://host/alice",
            false,
        )
        .expect("one instance");
        assert_eq!(shorthand[0].name, DEFAULT_SCROBBLE_DESTINATION);

        // Two names for one machine would give a member two links to keep in
        // step with one profile, and each would look like a separate
        // destination in every counter.
        assert!(parse_destinations(
            crate::services::ScrobbleProvider::Maloja,
            "WAVEFLOW_SCROBBLE_MALOJA_URL",
            "alice=https://host/alice, ALICE=https://host/alice/",
            false,
        )
        .is_err());

        // And one name for two machines is not a configuration either.
        assert!(parse_destinations(
            crate::services::ScrobbleProvider::Maloja,
            "WAVEFLOW_SCROBBLE_MALOJA_URL",
            "alice=https://host/alice, alice=https://host/bob",
            false,
        )
        .is_err());
    }

    /// A name that cannot be a path segment is refused where it is written.
    #[test]
    fn a_destination_name_is_bounded_by_what_crosses_a_path() {
        for refused in [
            ".",
            "..",
            "al ice",
            "alice/bob",
            "alice%2f",
            "",
            &"a".repeat(65),
        ] {
            assert!(
                parse_destinations(
                    crate::services::ScrobbleProvider::Maloja,
                    "WAVEFLOW_SCROBBLE_MALOJA_URL",
                    &format!("{refused}=https://host/alice"),
                    false,
                )
                .is_err(),
                "{refused:?} must not become a path segment"
            );
        }
        for accepted in ["alice", "a-b_c.d", "ALICE2", &"a".repeat(64)] {
            assert!(
                parse_destinations(
                    crate::services::ScrobbleProvider::Maloja,
                    "WAVEFLOW_SCROBBLE_MALOJA_URL",
                    &format!("{accepted}=https://host/alice"),
                    false,
                )
                .is_ok(),
                "{accepted:?} must be usable"
            );
        }
    }
}
