//! The generic readers every setting goes through: one variable, parsed or
//! refused.

/// One environment variable, trimmed, with an empty value read as absent.
///
/// "Set to nothing" and "not set" are the same statement from an operator, and
/// an `.env` file that keeps a commented-out key as `NAME=` is the ordinary way
/// of making it.
pub(super) fn read_trimmed_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub(super) fn parse_env<T>(name: &str, default: &str) -> anyhow::Result<T>
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
pub(super) fn parse_bool_env(name: &str, default: bool) -> anyhow::Result<bool> {
    parse_bool_value(name, std::env::var(name).ok().as_deref(), default)
}

/// [`parse_bool_env`] over a value already read, for a setting resolved from
/// a lookup the tests can hand in instead of the process environment
/// (`laptop.rs`).
pub(super) fn parse_bool_value(
    name: &str,
    raw: Option<&str>,
    default: bool,
) -> anyhow::Result<bool> {
    let Some(raw) = raw else {
        return Ok(default);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "" => Ok(default),
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        other => anyhow::bail!("invalid {name}: expected a boolean, found {other:?}"),
    }
}

pub(super) fn parse_positive_env<T>(name: &str, default: T) -> anyhow::Result<T>
where
    T: std::str::FromStr + PartialOrd + Default + Copy + std::fmt::Display,
    T::Err: std::fmt::Display,
{
    parse_positive_value(name, std::env::var(name).ok(), default)
}

/// [`parse_positive_env`] over a value already read, as [`parse_bool_value`]
/// is.
pub(super) fn parse_positive_value<T>(
    name: &str,
    raw: Option<String>,
    default: T,
) -> anyhow::Result<T>
where
    T: std::str::FromStr + PartialOrd + Default + Copy + std::fmt::Display,
    T::Err: std::fmt::Display,
{
    let raw = raw.unwrap_or_else(|| default.to_string());
    let value = raw
        .parse::<T>()
        .map_err(|error| anyhow::anyhow!("invalid {name}: {error}"))?;
    if value <= T::default() {
        anyhow::bail!("invalid {name}: must be greater than zero");
    }
    Ok(value)
}
