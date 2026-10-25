//! Podcast feeds: the one outbound fetch whose addresses come from a person,
//! and the parser that reads what comes back (RFC-025 decisions 8 to 10).
//!
//! Every other outbound request this server makes goes to a host an operator
//! named or a constant names. A feed address is typed by an administrator, and
//! every address after it — the redirects, the enclosures — is read out of a
//! response, so each one is treated as untrusted:
//!
//! - **http or https only**, and http only where the operator allows it
//!   (`WAVEFLOW_PODCAST_ALLOW_HTTP`). No user information in the address.
//! - **Every address the name resolves to is public**, by the same policy the
//!   portrait fetcher uses ([`AddressPolicy`]), checked **before connecting**
//!   and pinned, so the address checked is the address connected to.
//! - **Redirects are followed by hand, at most [`MAX_REDIRECTS`]**, and each
//!   hop is put through every rule above before it is contacted. The shared
//!   client follows none: a `302` to `http://169.254.169.254/` is refused as an
//!   address, never fetched.
//! - **Bodies are bounded while they are read**, and the whole request by a
//!   deadline.
//! - **Nothing about a request is logged** but a verdict from a fixed list: a
//!   feed address may carry a private token in its path or its query string.
//!
//! Parsing goes through `roxmltree` with a DTD refused outright, so an entity
//! cannot be declared, let alone expanded, and with a node ceiling; the text
//! is bounded before it is parsed.

use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use crate::portrait::AddressPolicy;

/// How many redirects one fetch follows. Enclosures commonly pass through one
/// or two measurement prefixes on their way to a CDN; five leaves room for
/// that and for a feed that moved, and refuses a loop.
pub const MAX_REDIRECTS: usize = 5;

/// How many nodes a feed may parse into. A long-running show's feed holds a
/// few thousand items of a dozen nodes each; this is far above that and far
/// below what would exhaust memory.
const MAX_FEED_NODES: u32 = 1_000_000;

/// How many episodes one refresh keeps from a feed, newest first.
pub const MAX_EPISODES_PER_FEED: usize = 500;

/// The longest text kept from a title, in characters.
const MAX_TITLE_CHARS: usize = 500;

/// The longest text kept from a description, in characters.
const MAX_DESCRIPTION_CHARS: usize = 4_000;

/// The longest address kept, in characters.
const MAX_URL_CHARS: usize = 2_048;

/// The rules one fetch runs under.
#[derive(Debug, Clone, Copy)]
pub struct FetchPolicy {
    pub addresses: AddressPolicy,
    pub allow_http: bool,
    /// The whole request, redirects included.
    pub timeout: Duration,
}

/// Why a fetch did not produce a body. Carries nothing from the address or
/// the answer: [`Self::code`] is what is stored and shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchError {
    /// Not http(s), http where it is not allowed, user information, or no
    /// host. Nothing was contacted.
    Destination,
    /// The name resolved to an address the policy refuses. Nothing was
    /// contacted at that address.
    Address,
    /// More redirects than [`MAX_REDIRECTS`], or one without a usable target.
    Redirects,
    /// The body passed its ceiling.
    TooLarge,
    /// The name did not resolve, the connection failed, or the deadline passed.
    Unreached,
    /// The host answered something other than success.
    Status(u16),
}

impl FetchError {
    /// The word stored in `error_code`.
    pub fn code(self) -> String {
        match self {
            Self::Destination => "destination_refused".to_owned(),
            Self::Address => "address_refused".to_owned(),
            Self::Redirects => "too_many_redirects".to_owned(),
            Self::TooLarge => "too_large".to_owned(),
            Self::Unreached => "unreached".to_owned(),
            Self::Status(status) => format!("http_{status}"),
        }
    }
}

/// Whether `url` is an address a feed may be subscribed at: what the
/// subscription stores, judged before anything is fetched.
pub fn validate_feed_url(url: &str, allow_http: bool, policy: AddressPolicy) -> Option<url::Url> {
    let trimmed = url.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_URL_CHARS {
        return None;
    }
    let parsed = url::Url::parse(trimmed).ok()?;
    acceptable(&parsed, allow_http, policy).then_some(parsed)
}

/// The scheme and shape rules, before any name is resolved.
fn acceptable(url: &url::Url, allow_http: bool, policy: AddressPolicy) -> bool {
    let scheme_ok = match url.scheme() {
        "https" => true,
        // Plain http to loopback is what the test suite's listeners need, and
        // only under the policy that admits loopback at all.
        "http" => allow_http || policy == AddressPolicy::AllowLoopback,
        _ => false,
    };
    scheme_ok
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some_and(|host| !host.is_empty())
}

/// The address a host name stands for, if every address it resolves to is one
/// the policy admits.
async fn resolve(url: &url::Url, policy: AddressPolicy) -> Result<SocketAddr, FetchError> {
    let port = url.port_or_known_default().ok_or(FetchError::Destination)?;
    let addresses: Vec<SocketAddr> = match url.host() {
        Some(url::Host::Ipv4(ip)) => vec![SocketAddr::new(IpAddr::V4(ip), port)],
        Some(url::Host::Ipv6(ip)) => vec![SocketAddr::new(IpAddr::V6(ip), port)],
        Some(url::Host::Domain(host)) => tokio::net::lookup_host((host, port))
            .await
            .map_err(|_| FetchError::Unreached)?
            .collect(),
        None => return Err(FetchError::Destination),
    };
    // Every answer, not the first: a name that resolves to one public and one
    // private address is refused, whichever the client would have picked.
    if addresses.is_empty() {
        return Err(FetchError::Unreached);
    }
    if !addresses.iter().all(|address| policy.admits(address.ip())) {
        return Err(FetchError::Address);
    }
    Ok(addresses[0])
}

/// Opens `url` under every rule above, following redirects by hand, and
/// answers the response that carries the body.
///
/// Bounded by the caller's deadline, not by one of its own: a download and a
/// feed have different budgets and both include every hop.
pub async fn open(url: &url::Url, policy: FetchPolicy) -> Result<reqwest::Response, FetchError> {
    let mut current = url.clone();
    for _ in 0..=MAX_REDIRECTS {
        if !acceptable(&current, policy.allow_http, policy.addresses) {
            return Err(FetchError::Destination);
        }
        let pinned = resolve(&current, policy.addresses).await?;
        // Plain http reaches loopback only, whatever the switch says, while the
        // policy is the test suite's: it is the one place a refused address
        // could otherwise be reached in clear by name.
        if current.scheme() == "http" && !policy.allow_http && !pinned.ip().is_loopback() {
            return Err(FetchError::Destination);
        }
        let host = current
            .host_str()
            .ok_or(FetchError::Destination)?
            .to_owned();
        let mut builder = crate::outbound::builder(policy.timeout);
        if matches!(current.host(), Some(url::Host::Domain(_))) {
            builder = builder.resolve(&host, pinned);
        }
        let client = builder.build().map_err(|_| FetchError::Unreached)?;
        let response = client
            .get(current.clone())
            .send()
            .await
            .map_err(|_| FetchError::Unreached)?;
        let status = response.status();
        if status.is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or(FetchError::Redirects)?;
            current = current.join(location).map_err(|_| FetchError::Redirects)?;
            continue;
        }
        if !status.is_success() {
            return Err(FetchError::Status(status.as_u16()));
        }
        return Ok(response);
    }
    Err(FetchError::Redirects)
}

/// A whole body, at most `max_bytes`, within the policy's deadline. The bound
/// is applied to the declared length first and then to every chunk, so a host
/// that lies about its length is stopped where it passes the ceiling.
pub async fn fetch_bounded(
    url: &url::Url,
    policy: FetchPolicy,
    max_bytes: usize,
) -> Result<Vec<u8>, FetchError> {
    let within = async {
        let mut response = open(url, policy).await?;
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes as u64)
        {
            return Err(FetchError::TooLarge);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| FetchError::Unreached)? {
            if bytes.len() + chunk.len() > max_bytes {
                return Err(FetchError::TooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    };
    tokio::time::timeout(policy.timeout, within)
        .await
        .unwrap_or(Err(FetchError::Unreached))
}

/// A channel, as its feed describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct Feed {
    pub title: String,
    pub description: Option<String>,
    pub site_url: Option<String>,
    pub image_url: Option<String>,
    /// Newest first, at most [`MAX_EPISODES_PER_FEED`].
    pub episodes: Vec<FeedEpisode>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeedEpisode {
    pub guid: String,
    pub title: String,
    pub description: Option<String>,
    pub published_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub enclosure_url: String,
    pub enclosure_type: Option<String>,
    pub enclosure_length: Option<i64>,
}

/// Why a body is not a feed. One code, `invalid_feed`, is all that is stored:
/// the parser's own message would quote the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedError {
    /// Not text, not XML, a DTD, too many nodes, or neither RSS nor Atom.
    Invalid,
}

impl FeedError {
    pub fn code(self) -> String {
        "invalid_feed".to_owned()
    }
}

/// Reads an RSS 2.0 or Atom document.
///
/// `base` is where the document was fetched from, against which a relative
/// enclosure address is resolved. An item without an http(s) enclosure is not
/// an episode and is skipped.
pub fn parse_feed(bytes: &[u8], base: &url::Url) -> Result<Feed, FeedError> {
    let text = decode(bytes).ok_or(FeedError::Invalid)?;
    let options = roxmltree::ParsingOptions {
        // The default, written down because it is the protection: a document
        // declaring a DTD is refused, so no entity can be defined to expand.
        allow_dtd: false,
        nodes_limit: MAX_FEED_NODES,
        ..roxmltree::ParsingOptions::default()
    };
    let document =
        roxmltree::Document::parse_with_options(&text, options).map_err(|_| FeedError::Invalid)?;
    let root = document.root_element();
    let mut feed = match root.tag_name().name() {
        "rss" => {
            let channel = child(root, "channel").ok_or(FeedError::Invalid)?;
            parse_rss(channel, base)
        }
        "feed" => parse_atom(root, base),
        // RSS 1.0 (RDF): channel and items are siblings under the root.
        "RDF" => parse_rdf(root, base),
        _ => return Err(FeedError::Invalid),
    };
    feed.episodes
        .sort_by_key(|episode| std::cmp::Reverse(episode.published_at));
    let mut seen = std::collections::HashSet::new();
    feed.episodes
        .retain(|episode| seen.insert(episode.guid.clone()));
    feed.episodes.truncate(MAX_EPISODES_PER_FEED);
    Ok(feed)
}

/// The document as text: a byte-order mark first, then the encoding the XML
/// declaration names, then UTF-8.
fn decode(bytes: &[u8]) -> Option<String> {
    if let Some((encoding, _)) = encoding_rs::Encoding::for_bom(bytes) {
        let (text, _, malformed) = encoding.decode(bytes);
        return (!malformed).then(|| text.into_owned());
    }
    let head = &bytes[..bytes.len().min(256)];
    // Lossy, because only the ASCII of the declaration matters here and the
    // cut at 256 bytes may fall inside a character.
    let head = String::from_utf8_lossy(head);
    let declared = (|| {
        let start = head.find("encoding=")? + "encoding=".len();
        let quote = head[start..].chars().next()?;
        let rest = &head[start + 1..];
        let end = rest.find(quote)?;
        Some(rest[..end].to_owned())
    })();
    let encoding = declared
        .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    let (text, malformed) = encoding.decode_without_bom_handling(bytes);
    // A feed whose bytes are not what it says it is, is not read: guessing
    // would store mojibake as a title.
    (!malformed).then(|| text.into_owned())
}

fn child<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children()
        .find(|candidate| candidate.is_element() && candidate.tag_name().name() == name)
}

/// The text of the first child called `name` in no namespace or the RSS one,
/// trimmed, `None` when empty.
fn text_of(node: roxmltree::Node<'_, '_>, name: &str, limit: usize) -> Option<String> {
    node.children()
        .filter(|candidate| candidate.is_element() && candidate.tag_name().name() == name)
        .map(|candidate| collect_text(candidate, limit))
        .find(|text| !text.is_empty())
}

/// Every text descendant, markup removed, whitespace collapsed, bounded.
///
/// Iterative, through `descendants`, so a deeply nested document cannot drive
/// this into the stack. HTML inside a CDATA section arrives as text and is
/// stripped of its tags here as well: what a client is handed is a plain
/// sentence, never markup a feed chose.
fn collect_text(node: roxmltree::Node<'_, '_>, limit: usize) -> String {
    let mut raw = String::new();
    for descendant in node.descendants() {
        if descendant.is_text() {
            if let Some(text) = descendant.text() {
                raw.push_str(text);
                raw.push(' ');
                if raw.len() > limit * 8 {
                    break;
                }
            }
        }
    }
    plain_text(&raw, limit)
}

/// Markup removed, entities left as the parser decoded them, whitespace
/// collapsed, at most `limit` characters.
fn plain_text(raw: &str, limit: usize) -> String {
    let mut out = String::new();
    // Counted as they are pushed: `out.chars().count()` on every character
    // was quadratic in the limit, and ran once per text of every item.
    let mut pushed = 0usize;
    let mut in_tag = false;
    let mut last_space = true;
    for character in raw.chars() {
        match character {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                if !last_space {
                    out.push(' ');
                    pushed += 1;
                    last_space = true;
                }
            }
            _ if in_tag => {}
            c if c.is_whitespace() => {
                if !last_space {
                    out.push(' ');
                    pushed += 1;
                    last_space = true;
                }
            }
            c if c.is_control() => {}
            c => {
                out.push(c);
                pushed += 1;
                last_space = false;
            }
        }
        if pushed >= limit {
            break;
        }
    }
    out.trim().chars().take(limit).collect()
}

/// An address out of the document, resolved against `base`, kept only when it
/// is absolute http(s) and of a sensible length.
fn address(value: &str, base: &url::Url) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > MAX_URL_CHARS {
        return None;
    }
    let url = base.join(value).ok()?;
    matches!(url.scheme(), "http" | "https")
        .then(|| url.to_string())
        .filter(|url| url.chars().count() <= MAX_URL_CHARS)
}

fn itunes_image(node: roxmltree::Node<'_, '_>, base: &url::Url) -> Option<String> {
    node.children()
        .find(|candidate| candidate.is_element() && candidate.tag_name().name() == "image")
        .and_then(|image| {
            image
                .attribute("href")
                .and_then(|href| address(href, base))
                .or_else(|| text_of(image, "url", MAX_URL_CHARS).and_then(|u| address(&u, base)))
        })
}

fn parse_rss(channel: roxmltree::Node<'_, '_>, base: &url::Url) -> Feed {
    let episodes = channel
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "item")
        .filter_map(|item| rss_item(item, base))
        .collect();
    Feed {
        title: text_of(channel, "title", MAX_TITLE_CHARS).unwrap_or_else(|| "Untitled".to_owned()),
        description: text_of(channel, "description", MAX_DESCRIPTION_CHARS)
            .or_else(|| text_of(channel, "summary", MAX_DESCRIPTION_CHARS)),
        site_url: text_of(channel, "link", MAX_URL_CHARS).and_then(|link| address(&link, base)),
        image_url: itunes_image(channel, base),
        episodes,
    }
}

fn rss_item(item: roxmltree::Node<'_, '_>, base: &url::Url) -> Option<FeedEpisode> {
    let enclosure = child(item, "enclosure")?;
    let enclosure_url = address(enclosure.attribute("url")?, base)?;
    let guid = text_of(item, "guid", MAX_URL_CHARS).unwrap_or_else(|| enclosure_url.clone());
    Some(FeedEpisode {
        guid,
        title: text_of(item, "title", MAX_TITLE_CHARS).unwrap_or_else(|| "Untitled".to_owned()),
        description: text_of(item, "description", MAX_DESCRIPTION_CHARS)
            .or_else(|| text_of(item, "summary", MAX_DESCRIPTION_CHARS)),
        published_at: text_of(item, "pubDate", 100)
            .or_else(|| text_of(item, "date", 100))
            .and_then(|date| parse_date(&date)),
        duration_ms: text_of(item, "duration", 32).and_then(|value| parse_duration(&value)),
        enclosure_type: enclosure
            .attribute("type")
            .map(|kind| kind.trim().chars().take(100).collect::<String>())
            .filter(|kind| !kind.is_empty()),
        enclosure_length: enclosure
            .attribute("length")
            .and_then(|length| length.trim().parse::<i64>().ok())
            .filter(|length| *length > 0),
        enclosure_url,
    })
}

fn parse_rdf(root: roxmltree::Node<'_, '_>, base: &url::Url) -> Feed {
    let channel = child(root, "channel");
    let episodes = root
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "item")
        .filter_map(|item| rss_item(item, base))
        .collect();
    Feed {
        title: channel
            .and_then(|channel| text_of(channel, "title", MAX_TITLE_CHARS))
            .unwrap_or_else(|| "Untitled".to_owned()),
        description: channel
            .and_then(|channel| text_of(channel, "description", MAX_DESCRIPTION_CHARS)),
        site_url: channel
            .and_then(|channel| text_of(channel, "link", MAX_URL_CHARS))
            .and_then(|link| address(&link, base)),
        image_url: None,
        episodes,
    }
}

/// The `href` of the first `link` with this `rel` (Atom's default is
/// `alternate`).
fn atom_link<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    rel: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children().find(|candidate| {
        candidate.is_element()
            && candidate.tag_name().name() == "link"
            && candidate.attribute("rel").unwrap_or("alternate") == rel
            && candidate.attribute("href").is_some()
    })
}

fn parse_atom(root: roxmltree::Node<'_, '_>, base: &url::Url) -> Feed {
    let episodes = root
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "entry")
        .filter_map(|entry| {
            let enclosure = atom_link(entry, "enclosure")?;
            let enclosure_url = address(enclosure.attribute("href")?, base)?;
            Some(FeedEpisode {
                guid: text_of(entry, "id", MAX_URL_CHARS).unwrap_or_else(|| enclosure_url.clone()),
                title: text_of(entry, "title", MAX_TITLE_CHARS)
                    .unwrap_or_else(|| "Untitled".to_owned()),
                description: text_of(entry, "summary", MAX_DESCRIPTION_CHARS)
                    .or_else(|| text_of(entry, "content", MAX_DESCRIPTION_CHARS)),
                published_at: text_of(entry, "published", 100)
                    .or_else(|| text_of(entry, "updated", 100))
                    .and_then(|date| parse_date(&date)),
                duration_ms: text_of(entry, "duration", 32)
                    .and_then(|value| parse_duration(&value)),
                enclosure_type: enclosure
                    .attribute("type")
                    .map(|kind| kind.trim().chars().take(100).collect::<String>())
                    .filter(|kind| !kind.is_empty()),
                enclosure_length: enclosure
                    .attribute("length")
                    .and_then(|length| length.trim().parse::<i64>().ok())
                    .filter(|length| *length > 0),
                enclosure_url,
            })
        })
        .collect();
    Feed {
        title: text_of(root, "title", MAX_TITLE_CHARS).unwrap_or_else(|| "Untitled".to_owned()),
        description: text_of(root, "subtitle", MAX_DESCRIPTION_CHARS),
        site_url: atom_link(root, "alternate")
            .and_then(|link| link.attribute("href"))
            .and_then(|href| address(href, base)),
        image_url: text_of(root, "logo", MAX_URL_CHARS)
            .or_else(|| text_of(root, "icon", MAX_URL_CHARS))
            .and_then(|logo| address(&logo, base))
            .or_else(|| itunes_image(root, base)),
        episodes,
    }
}

/// RFC 2822 (RSS), then RFC 3339 (Atom), in epoch milliseconds.
fn parse_date(value: &str) -> Option<i64> {
    let value = value.trim();
    chrono::DateTime::parse_from_rfc2822(value)
        .or_else(|_| chrono::DateTime::parse_from_rfc3339(value))
        .ok()
        .map(|date| date.timestamp_millis())
}

/// `itunes:duration`: seconds, `MM:SS` or `HH:MM:SS`.
fn parse_duration(value: &str) -> Option<i64> {
    let mut seconds: i64 = 0;
    let parts: Vec<&str> = value.trim().split(':').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    for part in parts {
        let number: f64 = part.trim().parse().ok()?;
        if !number.is_finite() || number < 0.0 {
            return None;
        }
        seconds = seconds.checked_mul(60)?.checked_add(number as i64)?;
    }
    seconds.checked_mul(1000).filter(|ms| *ms < 100 * 3_600_000)
}

/// The audio formats an episode may be stored as, recognised from the bytes:
/// `(suffix, content type)`. Anything else is refused — a feed that serves a
/// page of HTML, or a binary, as its enclosure gets nothing written to disk.
pub fn sniff_audio(head: &[u8]) -> Option<(&'static str, &'static str)> {
    if head.starts_with(b"ID3")
        || (head.len() >= 2 && head[0] == 0xff && (head[1] & 0xe0) == 0xe0 && (head[1] & 0x06) != 0)
    {
        Some(("mp3", "audio/mpeg"))
    } else if head.len() >= 12 && &head[4..8] == b"ftyp" {
        Some(("m4a", "audio/mp4"))
    } else if head.starts_with(b"OggS") {
        Some(("ogg", "audio/ogg"))
    } else if head.starts_with(b"fLaC") {
        Some(("flac", "audio/flac"))
    } else if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WAVE" {
        Some(("wav", "audio/wav"))
    } else if head.len() >= 2 && head[0] == 0xff && (head[1] & 0xf6) == 0xf0 {
        // ADTS: twelve sync bits, layer zero.
        Some(("aac", "audio/aac"))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> url::Url {
        url::Url::parse("https://feeds.example.org/show.xml").unwrap()
    }

    #[test]
    fn an_rss_feed_is_read_newest_first_and_an_item_without_audio_is_skipped() {
        let feed = parse_feed(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd">
  <channel>
    <title>The Show</title>
    <description><![CDATA[<p>About <b>things</b></p>]]></description>
    <link>https://example.org/</link>
    <itunes:image href="https://example.org/cover.jpg"/>
    <item>
      <title>First</title>
      <guid>ep-1</guid>
      <pubDate>Mon, 01 Jan 2024 10:00:00 +0000</pubDate>
      <itunes:duration>1:02:03</itunes:duration>
      <enclosure url="/audio/1.mp3" type="audio/mpeg" length="1234"/>
    </item>
    <item>
      <title>Second</title>
      <pubDate>Tue, 02 Jan 2024 10:00:00 +0000</pubDate>
      <enclosure url="https://cdn.example.org/2.mp3" type="audio/mpeg"/>
    </item>
    <item><title>No audio</title></item>
  </channel>
</rss>"#,
            &base(),
        )
        .unwrap();
        assert_eq!(feed.title, "The Show");
        assert_eq!(feed.description.as_deref(), Some("About things"));
        assert_eq!(
            feed.image_url.as_deref(),
            Some("https://example.org/cover.jpg")
        );
        assert_eq!(feed.episodes.len(), 2);
        assert_eq!(feed.episodes[0].title, "Second");
        assert_eq!(feed.episodes[0].guid, "https://cdn.example.org/2.mp3");
        assert_eq!(
            feed.episodes[1].enclosure_url,
            "https://feeds.example.org/audio/1.mp3"
        );
        assert_eq!(feed.episodes[1].duration_ms, Some(3_723_000));
        assert_eq!(feed.episodes[1].enclosure_length, Some(1234));
    }

    #[test]
    fn an_atom_feed_is_read() {
        let feed = parse_feed(
            br#"<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Atom Show</title>
  <entry>
    <id>urn:1</id><title>One</title><updated>2024-03-01T00:00:00Z</updated>
    <link rel="enclosure" href="https://cdn.example.org/1.ogg" type="audio/ogg"/>
  </entry>
</feed>"#,
            &base(),
        )
        .unwrap();
        assert_eq!(feed.title, "Atom Show");
        assert_eq!(feed.episodes[0].guid, "urn:1");
        assert_eq!(feed.episodes[0].published_at, Some(1_709_251_200_000));
    }

    /// The billion laughs, and its quieter relatives: any DTD is refused, so no
    /// entity is declared and nothing expands.
    #[test]
    fn a_document_type_declaration_is_refused_outright() {
        let laughs = br#"<?xml version="1.0"?>
<!DOCTYPE lolz [<!ENTITY lol "lol"><!ENTITY lol2 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;">]>
<rss><channel><title>&lol2;</title></channel></rss>"#;
        assert_eq!(parse_feed(laughs, &base()), Err(FeedError::Invalid));
        let external = br#"<?xml version="1.0"?>
<!DOCTYPE rss [<!ENTITY xxe SYSTEM "file:///etc/passwd">]>
<rss><channel><title>&xxe;</title></channel></rss>"#;
        assert_eq!(parse_feed(external, &base()), Err(FeedError::Invalid));
        assert_eq!(parse_feed(b"not xml", &base()), Err(FeedError::Invalid));
        assert_eq!(
            parse_feed(b"<html><body/></html>", &base()),
            Err(FeedError::Invalid)
        );
    }

    #[test]
    fn an_enclosure_that_is_not_http_is_not_an_episode() {
        let feed = parse_feed(
            br#"<rss><channel><title>T</title>
<item><enclosure url="file:///etc/passwd"/></item>
<item><enclosure url="javascript:alert(1)"/></item>
</channel></rss>"#,
            &base(),
        )
        .unwrap();
        assert!(feed.episodes.is_empty());
    }

    #[test]
    fn a_feed_address_is_absolute_http_and_carries_no_credentials() {
        let public = AddressPolicy::Public;
        assert!(validate_feed_url("https://example.org/feed", false, public).is_some());
        assert!(validate_feed_url("http://example.org/feed", false, public).is_none());
        assert!(validate_feed_url("http://example.org/feed", true, public).is_some());
        assert!(validate_feed_url("https://user:secret@example.org/feed", true, public).is_none());
        assert!(validate_feed_url("file:///etc/passwd", true, public).is_none());
        assert!(validate_feed_url("ftp://example.org/feed", true, public).is_none());
        assert!(validate_feed_url("/feed.xml", true, public).is_none());
    }

    #[test]
    fn audio_is_recognised_by_its_bytes() {
        assert_eq!(sniff_audio(b"ID3\x04\0\0").map(|f| f.0), Some("mp3"));
        assert_eq!(
            sniff_audio(&[0xff, 0xfb, 0x90, 0x00]).map(|f| f.0),
            Some("mp3")
        );
        assert_eq!(sniff_audio(b"\0\0\0\x20ftypM4A ").map(|f| f.0), Some("m4a"));
        assert_eq!(sniff_audio(b"OggS\0\x02").map(|f| f.0), Some("ogg"));
        assert_eq!(sniff_audio(b"<html>").map(|f| f.0), None);
        assert_eq!(sniff_audio(b"MZ\x90\0").map(|f| f.0), None);
    }

    /// Text is cut where it passes its bound, exactly where the earlier count
    /// cut it. The count is now kept as characters are pushed rather than
    /// recomputed for each one; that recount was measured and is not slow in
    /// practice — std's character count runs optimized even in a debug build,
    /// and the feed above parses in well under a second either way — so this
    /// test holds the behaviour, not a timing.
    #[test]
    fn long_texts_are_cut_where_they_pass_their_bound() {
        let description = "word ".repeat(1_600);
        let items: String = (0..MAX_EPISODES_PER_FEED)
            .map(|index| {
                format!(
                    "<item><title>Episode {index}</title><description>{description}</description>\
                     <enclosure url=\"https://cdn.example.org/{index}.mp3\"/></item>"
                )
            })
            .collect();
        let document = format!("<rss><channel><title>Long</title>{items}</channel></rss>");
        let feed = parse_feed(document.as_bytes(), &base()).unwrap();
        assert_eq!(feed.episodes.len(), MAX_EPISODES_PER_FEED);
        let kept = feed.episodes[0].description.as_deref().unwrap();
        assert_eq!(
            kept.chars().count(),
            MAX_DESCRIPTION_CHARS - 1,
            "trimmed of its last space"
        );
        assert_eq!(plain_text("a <b>b</b>  c", 100), "a b c");
        assert_eq!(plain_text("abcdef", 3), "abc");
    }

    #[test]
    fn durations_take_the_three_spellings() {
        assert_eq!(parse_duration("90"), Some(90_000));
        assert_eq!(parse_duration("01:30"), Some(90_000));
        assert_eq!(parse_duration("1:00:00"), Some(3_600_000));
        assert_eq!(parse_duration("x"), None);
        assert_eq!(parse_duration("1:2:3:4"), None);
    }
}
