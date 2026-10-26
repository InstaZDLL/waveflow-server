//! Reading an `.m3u` or `.m3u8` playlist file.
//!
//! Pure: bytes in, entries out, no database and no filesystem. What an entry
//! *resolves to* is a catalogue question and lives in
//! [`crate::services`]; what an entry *says* is here.
//!
//! Two dialects, and the simple one is a subset of the other: a plain `.m3u`
//! is one target per line, and the extended one adds `#EXTM3U` on the first
//! line with an optional `#EXTINF` before each target. A reader that treats
//! every `#` line as a comment reads both, which is what this does — the
//! `#EXTINF` metadata is kept because it is the only thing left to match on
//! when a path resolves to nothing.

use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
use encoding_rs::Encoding;
use serde::{Serialize, Serializer};

/// How the bytes were read as text.
///
/// Reported rather than assumed, because the guess can be wrong and a wrong
/// guess mangles every title. A caller that sees `shift_jis` on a file it
/// believed was UTF-8 has been told something.
///
/// On the wire, the encoding's WHATWG name in lower case: `utf-8`,
/// `utf-16le`, `windows-1252`, `shift_jis`, `gbk`, `big5`, `euc-kr` and so on.
/// The two this answered before it read anything but UTF-8 and Windows-1252
/// are spelled as they were.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextEncoding(&'static Encoding);

impl TextEncoding {
    pub const UTF_8: Self = Self(encoding_rs::UTF_8);

    /// The name a report gives, `utf-8` rather than encoding_rs's `UTF-8`.
    pub fn name(self) -> String {
        self.0.name().to_ascii_lowercase()
    }
}

impl Serialize for TextEncoding {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.name())
    }
}

/// One line of the file that names something to play.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistFileEntry {
    /// One-based line number of the target, for a report that can be read
    /// against the file the user sent.
    pub line: usize,
    /// The target exactly as written, kept for the report: a user looking for
    /// why an entry failed needs to see what they wrote, not what we made of
    /// it.
    pub raw: String,
    /// The target reduced to path segments, separators folded and `.`/`..`
    /// resolved. Empty for a target that names no path at all — a bare URL.
    pub segments: Vec<String>,
    /// `#EXTINF` duration in seconds, when the file gave one.
    pub duration_secs: Option<i64>,
    /// `#EXTINF` display text, usually `Artist - Title`.
    pub title: Option<String>,
}

impl PlaylistFileEntry {
    /// `Artist - Title` split back into its two halves, when the title looks
    /// like one.
    ///
    /// The separator is " - " with spaces: a hyphen with no spaces around it
    /// belongs to the name more often than it separates two of them.
    pub fn artist_and_title(&self) -> Option<(&str, &str)> {
        let title = self.title.as_deref()?;
        let (artist, song) = title.split_once(" - ")?;
        let (artist, song) = (artist.trim(), song.trim());
        (!artist.is_empty() && !song.is_empty()).then_some((artist, song))
    }
}

/// A parsed playlist file.
#[derive(Debug, Clone)]
pub struct PlaylistFile {
    /// The name `#PLAYLIST` gave, when it gave one.
    pub name: Option<String>,
    pub encoding: TextEncoding,
    pub entries: Vec<PlaylistFileEntry>,
}

/// The file names more entries than the caller would take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooManyEntries;

/// Reads the file, up to `max_entries` entries.
///
/// Fails on one thing only: a file naming more than `max_entries` entries,
/// noticed at the first entry past the line, before it is built (RFC-023
/// decision 5). The caller's ceiling used to be applied once the whole file
/// was read, and a five-megabyte body of one-letter lines is millions of
/// entries, each with its own strings, before anything refused it.
///
/// Otherwise lenient: a line this does not understand is a line it reports
/// nothing about, and an empty result is an empty playlist rather than an
/// error. The caller decides what an empty import means, which is a policy
/// question and not a parsing one.
pub fn parse(bytes: &[u8], max_entries: usize) -> Result<PlaylistFile, TooManyEntries> {
    let (text, encoding) = decode(bytes);
    let mut name = None;
    let mut pending: Option<(Option<i64>, Option<String>)> = None;
    let mut entries = Vec::new();
    for (index, raw_line) in text.lines().enumerate() {
        // A `.m3u` written on Windows and read here keeps its carriage
        // returns, and a trailing `\r` in a path matches nothing.
        let line = raw_line
            .trim_matches(|c: char| c == '\r' || c == '\n')
            .trim();
        if line.is_empty() {
            continue;
        }
        if let Some(directive) = line.strip_prefix('#') {
            if let Some(value) = directive.strip_prefix("EXTINF:") {
                pending = Some(parse_extinf(value));
            } else if let Some(value) = directive.strip_prefix("PLAYLIST:") {
                let value = value.trim();
                if !value.is_empty() {
                    name = Some(value.to_owned());
                }
            }
            // Everything else — `#EXTM3U`, `#EXTALB`, `#EXTGRP`, a human
            // comment — says nothing this import can act on.
            continue;
        }
        if entries.len() == max_entries {
            return Err(TooManyEntries);
        }
        let (duration_secs, title) = pending.take().unwrap_or((None, None));
        entries.push(PlaylistFileEntry {
            line: index + 1,
            raw: line.to_owned(),
            segments: segments(line),
            duration_secs,
            title,
        });
    }
    Ok(PlaylistFile {
        name,
        encoding,
        entries,
    })
}

/// The text, read by `waveflow_core::text_encoding`, and the encoding it was
/// read as.
///
/// `.m3u8` is UTF-8 by definition and `.m3u` frequently is not, but the
/// extension is a claim and the bytes are evidence, so the bytes decide. The
/// core decoder is the one the desktop reads the same files with: a
/// byte-order mark names the encoding, valid UTF-8 is UTF-8, and anything
/// else has its encoding guessed. A fixed fallback, Windows-1252 as this was
/// until the core had a decoder, read a CJK playlist into Latin nonsense and
/// a UTF-16 one into NULs.
fn decode(bytes: &[u8]) -> (String, TextEncoding) {
    let text = waveflow_core::text_encoding::decode(bytes).into_owned();
    (text, TextEncoding(encoding_read_as(bytes)))
}

/// The encoding `waveflow_core::text_encoding::decode` reads `bytes` as.
///
/// The core answers the text and not its encoding, so the encoding is worked
/// out again here by the same steps in the same order, with the detector set
/// up the same way. Duplicated rather than trusted: a core that changed its
/// guess would have a report name one encoding while the text was read as
/// another, which is what `the_encoding_named_is_the_one_the_core_read`
/// catches on the next bump.
fn encoding_read_as(bytes: &[u8]) -> &'static Encoding {
    if let Some((encoding, _)) = Encoding::for_bom(bytes) {
        return encoding;
    }
    if std::str::from_utf8(bytes).is_ok() {
        return encoding_rs::UTF_8;
    }
    let mut detector = EncodingDetector::new(Iso2022JpDetection::Deny);
    detector.feed(bytes, true);
    detector.guess(None, Utf8Detection::Deny)
}

/// `#EXTINF:duration,display text`.
///
/// The duration is `-1` for a stream and may carry attributes after it in the
/// HLS dialect; neither is this import's business, so anything that is not a
/// positive integer becomes `None` rather than an error.
fn parse_extinf(value: &str) -> (Option<i64>, Option<String>) {
    let (duration, title) = match value.split_once(',') {
        Some((duration, title)) => (duration.trim(), title.trim()),
        None => (value.trim(), ""),
    };
    let duration = duration
        .split_whitespace()
        .next()
        .and_then(|head| head.parse::<i64>().ok())
        .filter(|seconds| *seconds > 0);
    let title = (!title.is_empty()).then(|| title.to_owned());
    (duration, title)
}

/// A target reduced to the segments a path comparison can use.
///
/// Four shapes arrive, and all four are the same shape underneath:
/// `Artist/Album/01.flac`, `../Artist/Album/01.flac`,
/// `/home/someone/Music/Artist/Album/01.flac` and
/// `C:\Users\someone\Music\Artist\Album\01.flac`. What they share is the tail,
/// so the leading part is simply kept as segments and the comparison takes the
/// tail it needs. A `file://` URL is percent-decoded first; any other scheme
/// yields nothing, because a playlist entry pointing at a web radio is not a
/// track this catalogue holds.
///
/// The scheme is matched without regard to case, as RFC 3986 defines it: a
/// player that writes `File://` names the same thing, and reading it as an
/// unknown scheme would have reported the entry as naming no path at all.
fn segments(target: &str) -> Vec<String> {
    let path = match target.split_once("://") {
        None => percent_decode(target),
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("file") => {
            // `file:///C:/x` and `file://host/x`: the authority is dropped
            // either way, because neither names anything here.
            let rest = rest.split_once('/').map_or("", |(_, tail)| tail);
            percent_decode(rest)
        }
        Some(_) => return Vec::new(),
    };
    let mut out: Vec<String> = Vec::new();
    for segment in path.split(['/', '\\']) {
        match segment {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other.to_owned()),
        }
    }
    out
}

/// `%20` and friends, decoded leniently.
///
/// A lone `%` or a truncated escape is kept as written rather than dropped: it
/// is almost certainly a literal percent in a file name, and an entry that
/// fails to resolve is reported with its raw text, so guessing wrong here is
/// visible rather than silent.
fn percent_decode(value: &str) -> String {
    percent_encoding::percent_decode_str(value)
        .decode_utf8()
        .map(|decoded| decoded.into_owned())
        .unwrap_or_else(|_| value.to_owned())
}

#[cfg(test)]
mod tests;
