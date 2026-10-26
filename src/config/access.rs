//! How the server is reached: the origin it is published at, the proxies whose
//! forwarding it believes, and the token that guards first-run setup.

/// The shortest operator-chosen setup token accepted.
///
/// The drawn one carries 256 bits. This is the floor for a value somebody
/// typed, and nothing throttles setup: a guess at the token costs one request.
pub const SETUP_TOKEN_MIN_CHARS: usize = 16;

/// `WAVEFLOW_SETUP_TOKEN`, refused when too short to stand unthrottled.
pub(super) fn parse_setup_token(value: Option<String>) -> anyhow::Result<Option<String>> {
    match value {
        Some(token) if token.chars().count() < SETUP_TOKEN_MIN_CHARS => anyhow::bail!(
            "WAVEFLOW_SETUP_TOKEN must be at least {SETUP_TOKEN_MIN_CHARS} characters"
        ),
        other => Ok(other),
    }
}

pub(super) fn normalize_public_url(value: &str) -> anyhow::Result<String> {
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

/// `WAVEFLOW_TRUSTED_PROXIES`: addresses or CIDR blocks. A bare address is its
/// own block. A typo refuses to boot rather than trusting less, or more, than
/// the operator wrote. A block written in IPv4-mapped form is read as the IPv4
/// block it spells, which is how the limiter compares it.
pub(super) fn parse_proxy_addresses(list: &str) -> anyhow::Result<Vec<ipnet::IpNet>> {
    list.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            entry
                .parse::<ipnet::IpNet>()
                .or_else(|_| entry.parse::<std::net::IpAddr>().map(ipnet::IpNet::from))
                .map(crate::sign_in::trusted_network)
                .map_err(|_| anyhow::anyhow!("invalid WAVEFLOW_TRUSTED_PROXIES entry {entry:?}"))
        })
        .collect()
}
