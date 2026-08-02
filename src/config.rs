//! WaveFlow v2 process configuration.
//!
//! Environment access is centralised here so domain and repository code stay
//! deterministic and testable.

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
    pub sqlite_busy_timeout: Duration,
    pub access_token_ttl: Duration,
    pub refresh_token_ttl: Duration,
    pub scan_interval: Option<Duration>,
    pub scan_parallelism: usize,
    pub ffmpeg_path: PathBuf,
    pub ffprobe_path: PathBuf,
    pub transcode_cache_dir: PathBuf,
    pub transcode_cache_max_bytes: u64,
    pub transcode_global_limit: usize,
    pub transcode_per_user_limit: usize,
    pub allowed_origins: Vec<axum::http::HeaderValue>,
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
            .field("refresh_token_ttl", &self.refresh_token_ttl)
            .field("scan_interval", &self.scan_interval)
            .field("scan_parallelism", &self.scan_parallelism)
            .field("ffmpeg_path", &self.ffmpeg_path)
            .field("ffprobe_path", &self.ffprobe_path)
            .field("transcode_cache_dir", &self.transcode_cache_dir)
            .field("transcode_cache_max_bytes", &self.transcode_cache_max_bytes)
            .field("transcode_global_limit", &self.transcode_global_limit)
            .field("transcode_per_user_limit", &self.transcode_per_user_limit)
            .field("allowed_origins", &self.allowed_origins)
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

        if refresh_token_ttl_secs <= access_token_ttl_secs {
            anyhow::bail!(
                "WAVEFLOW_REFRESH_TOKEN_TTL_SECS must be greater than WAVEFLOW_ACCESS_TOKEN_TTL_SECS"
            );
        }
        let transcode_cache_dir = data_dir.join("transcode-cache");

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
            allowed_origins,
        })
    }

    pub fn for_data_dir(data_dir: PathBuf) -> Self {
        let transcode_cache_dir = data_dir.join("transcode-cache");
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
            refresh_token_ttl: Duration::from_secs(30 * 24 * 60 * 60),
            scan_interval: None,
            scan_parallelism: 2,
            ffmpeg_path: PathBuf::from("ffmpeg"),
            ffprobe_path: PathBuf::from("ffprobe"),
            transcode_cache_dir,
            transcode_cache_max_bytes: 128 * 1024 * 1024,
            transcode_global_limit: 2,
            transcode_per_user_limit: 1,
            allowed_origins: Vec::new(),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::normalize_public_url;

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
