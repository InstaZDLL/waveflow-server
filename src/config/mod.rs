//! WaveFlow v2 process configuration.
//!
//! Environment access is centralised here so domain and repository code stay
//! deterministic and testable.

mod access;
mod env;
mod for_data_dir;
mod from_env;
mod limits;
mod metadata;
mod pid;
mod podcasts;
mod scrobbling;

pub use access::SETUP_TOKEN_MIN_CHARS;
pub use limits::{CanvasLimits, LibraryEventRetention, UploadLimits};
pub use metadata::{LyricsSettings, MetadataSettings};
pub use podcasts::{PodcastSettings, RemoteArtworkSettings};
pub use scrobbling::{
    LastFmApplication, ScrobbleDestination, ScrobbleLimits, DEFAULT_SCROBBLE_DESTINATION,
    DEFAULT_SCROBBLE_MAX_ATTEMPTS,
};

use std::{net::SocketAddr, path::PathBuf, time::Duration};

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
    /// How long taking a connection out of the pool may take before it gives
    /// up. `WAVEFLOW_DB_ACQUIRE_TIMEOUT_MS`, five seconds by default.
    ///
    /// **Every checkout, for the life of the process** — not only the boot.
    /// sqlx applies it to each `acquire`, so it covers creating the file and
    /// establishing the connection the migrations run on, and equally a
    /// request waiting for a free connection once every one of
    /// `db_max_connections` is busy. It does **not** bound the SQL that runs
    /// once a connection is in hand; lock contention between writers is
    /// `sqlite_busy_timeout`.
    ///
    /// So raising it trades failing fast for waiting: a saturated server
    /// holds a request longer instead of refusing it. Five seconds is ample
    /// on a warm disk — sqlx's own default is thirty — and has proved too
    /// tight on a loaded CI runner, where three separate test targets have
    /// failed at `Database::open` with "pool timed out while waiting for an
    /// open connection". Either way the only thing this can produce is a
    /// timeout, never a wrong answer, and a deployment on slow or contended
    /// storage can raise it rather than fail to start.
    pub db_acquire_timeout: Duration,
    pub sqlite_busy_timeout: Duration,
    pub access_token_ttl: Duration,
    /// Lifetime of a browser stream ticket. It must outlive a full listen, not
    /// just the initial request: the browser reuses the same URL for every
    /// range request, so a seek late in a long track still redeems the original
    /// ticket. Access is re-checked on every redemption, so this bounds how
    /// long a leaked URL stays useful, not how long access itself lasts.
    pub stream_ticket_ttl: Duration,
    pub refresh_token_ttl: Duration,
    /// How long a client credential an **account** mints for itself may live.
    /// `WAVEFLOW_API_TOKEN_TTL_SECS`, a year by default; `0` for no expiry.
    ///
    /// It exists because the self-service route changed what a stolen session
    /// can leave behind. Before it, minting a token needed the instance's
    /// administrator, so a session taken for the fifteen minutes of its access
    /// token could read a library and nothing more. Now it can register a
    /// credential — and `api_token` has always been written without an
    /// `expires_at`, so that credential outlived the theft by as long as
    /// nobody noticed.
    ///
    /// The administrative route is deliberately **not** bounded by this. An
    /// operator provisioning a service account is a different act from a
    /// person clicking a button in their settings: it carries an audit entry,
    /// it needs a role no stolen session can reach, and a token that quietly
    /// stops working in a year is an operational surprise an operator did not
    /// ask for.
    pub api_token_ttl: Option<Duration>,
    pub scan_interval: Option<Duration>,
    pub scan_parallelism: usize,
    /// Whether the server watches every library root and scans a library once
    /// its changes settle (RFC-021 decision 6). `WAVEFLOW_WATCH`, on by
    /// default. The periodic scan stays either way, as the safety net for
    /// what a watcher misses — network mounts, above all.
    pub watch: bool,
    /// How long a library's changes must have been quiet before the watcher
    /// scans it. `WAVEFLOW_WATCH_DELAY_SECS`, ten seconds by default. A stream
    /// of changes that never settles is still scanned once thirty of these
    /// have passed since its first event.
    pub watch_delay: Duration,
    /// How long a track may stay unavailable before a background pass purges
    /// it (RFC-021 decision 4). `WAVEFLOW_UNAVAILABLE_RETENTION_DAYS`; unset or
    /// `0`, the default, never purges on its own: a purge is not reversible,
    /// and a share that stays unmounted for a week is not a deletion.
    pub unavailable_retention: Option<Duration>,
    /// How long a purge of missing tracks can be taken back before their rows
    /// are deleted for good (RFC-021, undoing a purge).
    /// `WAVEFLOW_PURGE_GRACE_HOURS`, a week by default; `0` deletes at once,
    /// as a purge did before there was a way back.
    pub purge_grace: Duration,
    /// Folders outside every library that a followed symbolic link may point
    /// into (RFC-021, links outside the root), for a link farm whose files
    /// live elsewhere — a beets `link: yes` import, say.
    /// `WAVEFLOW_LINK_ROOTS`, absolute paths separated as `PATH` is (`:` on
    /// Unix, `;` on Windows); empty by default. Read only by a library whose
    /// links setting is on, and resolved again at every scan, so a mount that
    /// appears later is honoured and one that vanished is skipped.
    pub link_roots: Vec<PathBuf>,
    /// The ReplayGain a Subsonic `stream` that is transcoded applies when the
    /// request does not say (RFC-022, what was still open).
    /// `WAVEFLOW_SUBSONIC_REPLAYGAIN`: `off`, the default, `track` or
    /// `album`. A request overrides it with `replayGain=`; the original file
    /// is never transcoded to apply it.
    pub subsonic_replay_gain: crate::replay_gain::ReplayGainMode,
    pub ffmpeg_path: PathBuf,
    pub ffprobe_path: PathBuf,
    pub transcode_cache_dir: PathBuf,
    pub transcode_cache_max_bytes: u64,
    pub transcode_global_limit: usize,
    pub transcode_per_user_limit: usize,
    /// How many lossless files the quality analysis (RFC-016) measures at once,
    /// each one FFmpeg process on one thread. `WAVEFLOW_QUALITY_PARALLELISM`, `1`
    /// by default, so the analysis costs one core at most whatever the size of
    /// the library; `0` turns it off, and every lossless file then stays
    /// `pending`.
    pub quality_parallelism: usize,
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
    /// The Last.fm application, when the operator registered one.
    ///
    /// Last.fm is unavailable without it, and unavailable without an `https`
    /// `public_url` as well — the journey that brings the token back starts on
    /// the internet and crosses a person's browser, so decision 10's plaintext
    /// escape, which is about the operator's own network, does not reach it.
    /// `normalize_public_url` still accepts `http`, deliberately: the public URL
    /// also serves shares, where plaintext on a private network stays a
    /// legitimate operator's choice. The condition belongs to switching Last.fm
    /// on, not to the setting.
    pub lastfm: Option<LastFmApplication>,
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
    /// Metadata enrichment — RFC-011. See [`MetadataSettings`].
    pub metadata: MetadataSettings,
    /// Lyrics looked up on LRCLIB — RFC-024. See [`LyricsSettings`].
    pub lyrics: LyricsSettings,
    /// Podcasts — RFC-025. See [`PodcastSettings`].
    pub podcasts: PodcastSettings,
    /// Podcast covers and station images — RFC-025, amended. See
    /// [`RemoteArtworkSettings`].
    pub remote_artwork: RemoteArtworkSettings,
    pub allowed_origins: Vec<axum::http::HeaderValue>,
    /// `WAVEFLOW_TRUSTED_PROXIES`: the reverse proxies whose
    /// `X-Forwarded-For` is believed when telling one client from another
    /// (RFC-017 decision 2). Addresses or CIDR blocks, comma-separated, empty by
    /// default. Only a proxy that **appends** the address it received the
    /// connection from may be listed: one that forwards a client's header
    /// unchanged lets the client choose its address.
    pub trusted_proxies: Vec<ipnet::IpNet>,
    /// `WAVEFLOW_SETUP_TOKEN`: the one-time token `POST /api/v2/setup` asks
    /// for while no account exists (RFC-019 decision 1). Unset by default,
    /// and then the server draws one at boot and writes it to
    /// `<data dir>/setup-token`. An operator who sets it chooses the value
    /// instead, and no file is written; it must be at least
    /// [`SETUP_TOKEN_MIN_CHARS`] characters. Ignored once an account exists.
    pub setup_token: Option<String>,
    /// `WAVEFLOW_MIGRATION_BACKUPS`: how many copies of the database, taken
    /// with its key just before a boot applies pending migrations, are kept
    /// in the data directory (RFC-019 decision 3). `3` by default; `0` takes
    /// none. A fresh, empty database is never copied.
    pub migration_backups: usize,
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
            .field("db_acquire_timeout", &self.db_acquire_timeout)
            .field("sqlite_busy_timeout", &self.sqlite_busy_timeout)
            .field("access_token_ttl", &self.access_token_ttl)
            .field("stream_ticket_ttl", &self.stream_ticket_ttl)
            .field("refresh_token_ttl", &self.refresh_token_ttl)
            .field("api_token_ttl", &self.api_token_ttl)
            .field("scan_interval", &self.scan_interval)
            .field("scan_parallelism", &self.scan_parallelism)
            .field("watch", &self.watch)
            .field("watch_delay", &self.watch_delay)
            .field("unavailable_retention", &self.unavailable_retention)
            .field("purge_grace", &self.purge_grace)
            .field("link_roots", &self.link_roots)
            .field("subsonic_replay_gain", &self.subsonic_replay_gain)
            .field("ffmpeg_path", &self.ffmpeg_path)
            .field("ffprobe_path", &self.ffprobe_path)
            .field("transcode_cache_dir", &self.transcode_cache_dir)
            .field("transcode_cache_max_bytes", &self.transcode_cache_max_bytes)
            .field("transcode_global_limit", &self.transcode_global_limit)
            .field("quality_parallelism", &self.quality_parallelism)
            .field("transcode_per_user_limit", &self.transcode_per_user_limit)
            .field("uploads", &self.uploads)
            .field("library_event_retention", &self.library_event_retention)
            .field("canvas_dir", &self.canvas_dir)
            .field("canvas", &self.canvas)
            .field("scrobbling", &self.scrobbling)
            .field("destinations", &self.destinations)
            .field("lastfm", &self.lastfm)
            .field("outbound_allow_plaintext", &self.outbound_allow_plaintext)
            .field("metadata", &self.metadata)
            .field("lyrics", &self.lyrics)
            .field("podcasts", &self.podcasts)
            .field("remote_artwork", &self.remote_artwork)
            .field("allowed_origins", &self.allowed_origins)
            .field("trusted_proxies", &self.trusted_proxies)
            // Whether it is set, never what it is.
            .field(
                "setup_token",
                &self.setup_token.as_ref().map(|_| "<redacted>"),
            )
            .field("migration_backups", &self.migration_backups)
            .field("pid", &self.pid)
            .finish()
    }
}

#[cfg(test)]
mod tests;
