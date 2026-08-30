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
    /// What the server accepts when a member attaches a loop to a track.
    ///
    /// `WAVEFLOW_CANVAS_MAX_BYTES`, `WAVEFLOW_CANVAS_MAX_DURATION_SECS`,
    /// `WAVEFLOW_CANVAS_LIBRARY_QUOTA_BYTES`.
    ///
    /// Gated by the same `accepts_uploads` flag as a file: both answer "may a
    /// member of this library spend the operator's disk".
    pub canvas: CanvasLimits,
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
            .field("canvas_dir", &self.canvas_dir)
            .field("canvas", &self.canvas)
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
        if transcode_per_user_limit > transcode_global_limit {
            anyhow::bail!(
                "WAVEFLOW_TRANSCODE_PER_USER_LIMIT cannot exceed WAVEFLOW_TRANSCODE_GLOBAL_LIMIT"
            );
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
            canvas_dir,
            canvas,
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
            canvas_dir: canvas_dir_for_tests,
            // Same reasoning as the upload limits above: small enough that a
            // test can reach every bound, shaped like production rather than
            // unlimited.
            canvas: CanvasLimits {
                max_bytes: 256 * 1024,
                max_duration_secs: 15,
                library_quota_bytes: 1024 * 1024,
            },
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
}
