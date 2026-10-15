//! The narrow image fetcher of RFC-011 decision 8.
//!
//! A provider's answer names an image URL, and that URL is untrusted input: it
//! came out of a response, not out of the operator's configuration. So the
//! fetcher dereferences it only when every one of these holds:
//!
//! - **https**, and the host is on the provider's **allowlist**, compared
//!   exactly. A URL pointing anywhere else is dropped, not fetched.
//! - **Every address the name resolves to is public.** Loopback, private,
//!   link-local, shared, unspecified and multicast ranges are refused, IPv4
//!   inside IPv6 included.
//! - **The address checked is the address connected to.** The client is built
//!   for this one fetch with the checked address pinned
//!   (`ClientBuilder::resolve`), so the name is not looked up a second time by
//!   a resolver whose answer could differ.
//! - **No redirect** (the shared client's policy), a **size ceiling** enforced
//!   while reading, and a **content type** among the three the artwork route
//!   serves — confirmed by the bytes themselves, not taken from the header.
//!
//! The address policy is a parameter for one reason: the test suite's image
//! host lives on loopback, which the default refuses. [`AddressPolicy::default`]
//! is production, and a test asserts it refuses loopback, so the suite cannot
//! prove the fetcher only with its main protection switched off.

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::Path,
    time::Duration,
};

/// The largest portrait accepted. Deezer's largest size is a 1000×1000 JPEG,
/// well under a mebibyte; ten leaves room without letting a provider fill the
/// disk.
pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

/// Which addresses a fetch may connect to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AddressPolicy {
    /// Public addresses only, over https. Production.
    #[default]
    Public,
    /// Public addresses, and loopback over plain http as well. For the test
    /// suite's listeners, and for nothing else.
    AllowLoopback,
}

impl AddressPolicy {
    /// Whether a connection to `ip` is allowed.
    pub fn admits(self, ip: IpAddr) -> bool {
        let ip = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
            v4 => v4,
        };
        if ip.is_loopback() {
            return self == Self::AllowLoopback;
        }
        match ip {
            IpAddr::V4(v4) => is_public_v4(v4),
            IpAddr::V6(v6) => is_public_v6(v6),
        }
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    !(ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        // 100.64.0.0/10, carrier-grade NAT: another operator's private range.
        || (a == 100 && (64..128).contains(&b))
        // 0.0.0.0/8, "this network".
        || a == 0)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    !(ip.is_unspecified()
        || ip.is_multicast()
        // fc00::/7, unique local.
        || (first & 0xfe00) == 0xfc00
        // fe80::/10, link local.
        || (first & 0xffc0) == 0xfe80)
}

/// A fetched portrait, checked and hashed, not yet stored.
#[derive(Debug)]
pub struct Portrait {
    /// BLAKE3 of the bytes, hexadecimal: the artwork store's key.
    pub hash: String,
    /// `jpg`, `png` or `webp`, from the bytes.
    pub format: &'static str,
    pub bytes: Vec<u8>,
}

/// Why a portrait was not fetched. Carries nothing from the URL or the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Not https, or a host off the allowlist: the provider pointed somewhere
    /// it should not have. Nothing was contacted.
    Destination,
    /// The name resolved to an address the policy refuses. Nothing was
    /// contacted.
    Address,
    /// The answer is not one of the three image types, by header or by bytes.
    NotAnImage,
    /// Over [`MAX_IMAGE_BYTES`].
    TooLarge,
    /// The name did not resolve, the connection failed, or the deadline passed.
    Unreached,
    /// The host answered something other than success.
    Status(u16),
}

impl Refusal {
    /// Whether this says something about the image itself, which a cache may
    /// remember, rather than about the way to it, which the next visit should
    /// try again.
    pub fn is_answer(self) -> bool {
        matches!(
            self,
            Self::Destination | Self::Address | Self::NotAnImage | Self::TooLarge
        )
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Destination => formatter.write_str("image destination not allowed"),
            Self::Address => formatter.write_str("image address not allowed"),
            Self::NotAnImage => formatter.write_str("answer is not an image"),
            Self::TooLarge => formatter.write_str("image over the size ceiling"),
            Self::Unreached => formatter.write_str("image host unreached"),
            Self::Status(status) => write!(formatter, "image host answered HTTP {status}"),
        }
    }
}

/// Fetches `url` under every rule above, within `timeout` overall.
pub async fn fetch(
    url: &str,
    hosts: &[String],
    policy: AddressPolicy,
    timeout: Duration,
) -> Result<Portrait, Refusal> {
    tokio::time::timeout(timeout, fetch_within(url, hosts, policy, timeout))
        .await
        .unwrap_or(Err(Refusal::Unreached))
}

async fn fetch_within(
    url: &str,
    hosts: &[String],
    policy: AddressPolicy,
    timeout: Duration,
) -> Result<Portrait, Refusal> {
    let url = url::Url::parse(url).map_err(|_| Refusal::Destination)?;
    let plain = match url.scheme() {
        "https" => false,
        "http" if policy == AddressPolicy::AllowLoopback => true,
        _ => return Err(Refusal::Destination),
    };
    let host = url
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or(Refusal::Destination)?;
    if !hosts
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(&host))
    {
        return Err(Refusal::Destination);
    }
    let port = url.port_or_known_default().ok_or(Refusal::Destination)?;
    let addresses: Vec<SocketAddr> = match url.host() {
        Some(url::Host::Ipv4(ip)) => vec![SocketAddr::new(IpAddr::V4(ip), port)],
        Some(url::Host::Ipv6(ip)) => vec![SocketAddr::new(IpAddr::V6(ip), port)],
        _ => tokio::net::lookup_host((host.as_str(), port))
            .await
            .map_err(|_| Refusal::Unreached)?
            .collect(),
    };
    // Every answer, not the first: a name that resolves to one public and one
    // private address is refused, whichever the client would have picked.
    if addresses.is_empty() {
        return Err(Refusal::Unreached);
    }
    if !addresses.iter().all(|address| policy.admits(address.ip())) {
        return Err(Refusal::Address);
    }
    let pinned = addresses[0];
    if plain && !pinned.ip().is_loopback() {
        return Err(Refusal::Destination);
    }
    let client = crate::outbound::builder(timeout)
        .resolve(&host, pinned)
        .build()
        .map_err(|_| Refusal::Unreached)?;
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|_| Refusal::Unreached)?;
    if !response.status().is_success() {
        return Err(Refusal::Status(response.status().as_u16()));
    }
    let declared = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(|value| value.trim().to_ascii_lowercase())
        .and_then(|value| format_of_mime(&value))
        .ok_or(Refusal::NotAnImage)?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_IMAGE_BYTES as u64)
    {
        return Err(Refusal::TooLarge);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Refusal::Unreached)? {
        if bytes.len() + chunk.len() > MAX_IMAGE_BYTES {
            return Err(Refusal::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    let format = sniff(&bytes).ok_or(Refusal::NotAnImage)?;
    if format != declared {
        return Err(Refusal::NotAnImage);
    }
    Ok(Portrait {
        hash: blake3::hash(&bytes).to_hex().to_string(),
        format,
        bytes,
    })
}

fn format_of_mime(mime: &str) -> Option<&'static str> {
    match mime {
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

/// The format the bytes are, among the three the artwork route serves.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

/// Puts `portrait` in the artwork store, `<hash>.<format>` under
/// `artwork_dir`, unless it is already there.
///
/// Written beside the target and renamed into place, so a reader never sees
/// half a file. Called before the writer gate is taken: this is file I/O.
pub async fn store(artwork_dir: &Path, portrait: &Portrait) -> std::io::Result<()> {
    let target = artwork_dir.join(format!("{}.{}", portrait.hash, portrait.format));
    if tokio::fs::try_exists(&target).await? {
        return Ok(());
    }
    tokio::fs::create_dir_all(artwork_dir).await?;
    let staging = artwork_dir.join(format!("{}.part", uuid::Uuid::new_v4().simple()));
    tokio::fs::write(&staging, &portrait.bytes).await?;
    if let Err(error) = tokio::fs::rename(&staging, &target).await {
        let _ = tokio::fs::remove_file(&staging).await;
        // Another fill stored the same bytes first; the content is identical.
        if tokio::fs::try_exists(&target).await.unwrap_or(false) {
            return Ok(());
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The protection the suite cannot exercise end to end, since its image
    /// host is on loopback: the default refuses loopback and every private
    /// range, and admits a public address.
    #[test]
    fn the_default_policy_refuses_a_loopback_address() {
        let policy = AddressPolicy::default();
        for refused in [
            "127.0.0.1",
            "::1",
            "::ffff:127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::",
            "fd00::1",
            "fe80::1",
            "::ffff:192.168.1.1",
        ] {
            assert!(!policy.admits(refused.parse().unwrap()), "{refused}");
        }
        assert!(policy.admits("93.184.216.34".parse().unwrap()));
        assert!(policy.admits("2606:4700::1111".parse().unwrap()));
        assert!(AddressPolicy::AllowLoopback.admits("127.0.0.1".parse().unwrap()));
        assert!(!AddressPolicy::AllowLoopback.admits("10.0.0.1".parse().unwrap()));
    }

    #[test]
    fn the_bytes_decide_the_format() {
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0]), Some("jpg"));
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), Some("png"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(sniff(b"<html>"), None);
    }
}
