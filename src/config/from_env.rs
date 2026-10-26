//! `Config::from_env`: the process environment, read once and refused at boot
//! when it is wrong.

use std::{path::PathBuf, time::Duration};

use super::access::{normalize_public_url, parse_proxy_addresses, parse_setup_token};
use super::env::{parse_bool_env, parse_env, parse_positive_env, read_trimmed_env};
use super::limits::{validate_canvas, validate_uploads};
use super::metadata::{parse_lyrics_settings, parse_metadata_settings};
use super::pid::{
    parse_pid_spec, PidSpecKind, DEFAULT_PID_ALBUM, DEFAULT_PID_ARTIST, DEFAULT_PID_TRACK,
};
use super::podcasts::parse_podcast_settings;
use super::scrobbling::parse_destinations;
use super::{
    CanvasLimits, Config, LastFmApplication, LibraryEventRetention, RemoteArtworkSettings,
    ScrobbleLimits, UploadLimits, DEFAULT_SCROBBLE_MAX_ATTEMPTS,
};

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
        let db_acquire_timeout_ms = parse_positive_env(
            "WAVEFLOW_DB_ACQUIRE_TIMEOUT_MS",
            DEFAULT_DB_ACQUIRE_TIMEOUT_MS,
        )?;
        let sqlite_busy_timeout_ms =
            parse_positive_env("WAVEFLOW_SQLITE_BUSY_TIMEOUT_MS", 5_000u64)?;
        let stream_ticket_ttl_secs =
            parse_positive_env("WAVEFLOW_STREAM_TICKET_TTL_SECS", 60 * 60u64)?;
        let access_token_ttl_secs =
            parse_positive_env("WAVEFLOW_ACCESS_TOKEN_TTL_SECS", 15 * 60u64)?;
        let refresh_token_ttl_secs =
            parse_positive_env("WAVEFLOW_REFRESH_TOKEN_TTL_SECS", 30 * 24 * 60 * 60u64)?;
        // Zero is a value here and not a refusal — "no expiry" is a choice an
        // operator may make — so it is read like the scan interval rather than
        // through `parse_positive_env`.
        let api_token_ttl_secs = std::env::var("WAVEFLOW_API_TOKEN_TTL_SECS")
            .unwrap_or_else(|_| "31536000".to_owned())
            .parse::<u64>()
            .map_err(|error| anyhow::anyhow!("invalid WAVEFLOW_API_TOKEN_TTL_SECS: {error}"))?;
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
        let watch = parse_bool_env("WAVEFLOW_WATCH", true)?;
        let watch_delay =
            Duration::from_secs(parse_positive_env("WAVEFLOW_WATCH_DELAY_SECS", 10u64)?);
        // Zero is a setting, the default one: never purge on its own.
        let unavailable_retention_days = std::env::var("WAVEFLOW_UNAVAILABLE_RETENTION_DAYS")
            .unwrap_or_else(|_| "0".to_owned())
            .parse::<u64>()
            .map_err(|error| {
                anyhow::anyhow!("invalid WAVEFLOW_UNAVAILABLE_RETENTION_DAYS: {error}")
            })?;
        let unavailable_retention = (unavailable_retention_days > 0)
            .then(|| Duration::from_secs(unavailable_retention_days.saturating_mul(24 * 60 * 60)));
        // Zero is a setting too: no way back, as before.
        let purge_grace_hours = std::env::var("WAVEFLOW_PURGE_GRACE_HOURS")
            .unwrap_or_else(|_| "168".to_owned())
            .parse::<u64>()
            .map_err(|error| anyhow::anyhow!("invalid WAVEFLOW_PURGE_GRACE_HOURS: {error}"))?;
        let purge_grace = Duration::from_secs(purge_grace_hours.saturating_mul(60 * 60));
        let link_roots = parse_link_roots(std::env::var_os("WAVEFLOW_LINK_ROOTS"))?;
        let subsonic_replay_gain = match std::env::var("WAVEFLOW_SUBSONIC_REPLAYGAIN") {
            Err(_) => crate::media::replay_gain::ReplayGainMode::Off,
            Ok(raw) if raw.trim().is_empty() => crate::media::replay_gain::ReplayGainMode::Off,
            Ok(raw) => crate::media::replay_gain::ReplayGainMode::parse(&raw).ok_or_else(|| {
                anyhow::anyhow!(
                    "invalid WAVEFLOW_SUBSONIC_REPLAYGAIN: expected off, track or album, found {raw:?}"
                )
            })?,
        };
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
        // Zero is a setting here, not a mistake: it is how an operator turns the
        // analysis off, so this is not `parse_positive_env`.
        let quality_parallelism = std::env::var("WAVEFLOW_QUALITY_PARALLELISM")
            .unwrap_or_else(|_| "1".to_owned())
            .parse::<usize>()
            .map_err(|error| anyhow::anyhow!("invalid WAVEFLOW_QUALITY_PARALLELISM: {error}"))?;
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

        // Both or neither, and said at boot rather than discovered when
        // somebody tries to link: half an application is a typo, and a server
        // that started with Last.fm quietly switched off because of one is the
        // silent failure this whole RFC spends itself making visible.
        let lastfm = match (
            read_trimmed_env("WAVEFLOW_SCROBBLE_LASTFM_API_KEY"),
            read_trimmed_env("WAVEFLOW_SCROBBLE_LASTFM_SECRET"),
        ) {
            (None, None) => None,
            (Some(api_key), Some(secret)) => Some(LastFmApplication { api_key, secret }),
            (Some(_), None) => anyhow::bail!(
                "WAVEFLOW_SCROBBLE_LASTFM_API_KEY is set without WAVEFLOW_SCROBBLE_LASTFM_SECRET"
            ),
            (None, Some(_)) => anyhow::bail!(
                "WAVEFLOW_SCROBBLE_LASTFM_SECRET is set without WAVEFLOW_SCROBBLE_LASTFM_API_KEY"
            ),
        };

        let metadata = parse_metadata_settings()?;
        let lyrics = parse_lyrics_settings()?;
        let podcasts = parse_podcast_settings(&data_dir)?;
        let remote_artwork = RemoteArtworkSettings {
            enabled: parse_bool_env("WAVEFLOW_REMOTE_ARTWORK", true)?,
            timeout: Duration::from_secs(20),
            addresses: crate::remote::portrait::AddressPolicy::Public,
        };
        // Absent is empty; a value that is not UTF-8 refuses to boot, rather
        // than starting with no trusted proxy and counting every client behind
        // it under the proxy's address.
        let trusted_proxies = match std::env::var("WAVEFLOW_TRUSTED_PROXIES") {
            Ok(list) => parse_proxy_addresses(&list)?,
            Err(std::env::VarError::NotPresent) => Vec::new(),
            Err(std::env::VarError::NotUnicode(_)) => {
                anyhow::bail!("WAVEFLOW_TRUSTED_PROXIES is not valid UTF-8")
            }
        };

        let setup_token = parse_setup_token(read_trimmed_env("WAVEFLOW_SETUP_TOKEN"))?;
        // Zero is a setting, not a mistake: an operator with backups of their
        // own turns the copies off with it.
        let migration_backups = std::env::var("WAVEFLOW_MIGRATION_BACKUPS")
            .unwrap_or_else(|_| "3".to_owned())
            .parse::<usize>()
            .map_err(|error| anyhow::anyhow!("invalid WAVEFLOW_MIGRATION_BACKUPS: {error}"))?;

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
        let pid = crate::library::pid::PidSpecs {
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
            db_acquire_timeout: Duration::from_millis(db_acquire_timeout_ms),
            sqlite_busy_timeout: Duration::from_millis(sqlite_busy_timeout_ms),
            access_token_ttl: Duration::from_secs(access_token_ttl_secs),
            stream_ticket_ttl: Duration::from_secs(stream_ticket_ttl_secs),
            refresh_token_ttl: Duration::from_secs(refresh_token_ttl_secs),
            api_token_ttl: (api_token_ttl_secs > 0)
                .then(|| Duration::from_secs(api_token_ttl_secs)),
            scan_interval: (scan_interval_secs > 0)
                .then(|| Duration::from_secs(scan_interval_secs)),
            scan_parallelism,
            watch,
            watch_delay,
            unavailable_retention,
            purge_grace,
            link_roots,
            subsonic_replay_gain,
            ffmpeg_path,
            ffprobe_path,
            transcode_cache_dir,
            transcode_cache_max_bytes,
            transcode_global_limit,
            transcode_per_user_limit,
            quality_parallelism,
            uploads,
            library_event_retention,
            canvas_dir,
            canvas,
            scrobbling,
            destinations,
            lastfm,
            outbound_allow_plaintext,
            metadata,
            lyrics,
            podcasts,
            remote_artwork,
            allowed_origins,
            trusted_proxies,
            setup_token,
            migration_backups,
            pid,
        })
    }
}

/// `WAVEFLOW_LINK_ROOTS`: absolute folders, separated as `PATH` is.
///
/// Absolute because a relative one would be read against whatever directory
/// the server happens to start in, which is not a decision about the file
/// system an operator means to make. Not canonicalised here: a root on a
/// mount that is not there yet must not stop the boot, so the scan resolves
/// each one when it runs and skips one it cannot.
fn parse_link_roots(raw: Option<std::ffi::OsString>) -> anyhow::Result<Vec<PathBuf>> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let mut roots = Vec::new();
    for root in std::env::split_paths(&raw) {
        if root.as_os_str().is_empty() {
            continue;
        }
        if !root.is_absolute() {
            anyhow::bail!(
                "invalid WAVEFLOW_LINK_ROOTS: {} is not an absolute path",
                root.display()
            );
        }
        roots.push(root);
    }
    Ok(roots)
}

/// The acquire budget a deployment gets unless it says otherwise.
///
/// Named so the test fixture's longer budget can be compared against it
/// instead of against a second copy of the number.
pub(super) const DEFAULT_DB_ACQUIRE_TIMEOUT_MS: u64 = 5_000;
