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

use serde::Serialize;
use utoipa::ToSchema;

/// How the bytes were read as text.
///
/// Reported rather than assumed, because the guess can be wrong and a wrong
/// guess mangles every accented title. A caller that sees `Windows1252` on a
/// file it believed was UTF-8 has been told something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TextEncoding {
    // Named rather than derived, both of them: kebab-case puts no hyphen before
    // a digit, so the derive spelled these "utf8" and "windows1252" and no
    // encoding is called either.
    #[serde(rename = "utf-8")]
    Utf8,
    #[serde(rename = "windows-1252")]
    Windows1252,
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

/// Reads the file.
///
/// Never fails: a line this does not understand is a line it reports nothing
/// about, and an empty result is an empty playlist rather than an error. The
/// caller decides what an empty import means, which is a policy question and
/// not a parsing one.
pub fn parse(bytes: &[u8]) -> PlaylistFile {
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
        let (duration_secs, title) = pending.take().unwrap_or((None, None));
        entries.push(PlaylistFileEntry {
            line: index + 1,
            raw: line.to_owned(),
            segments: segments(line),
            duration_secs,
            title,
        });
    }
    PlaylistFile {
        name,
        encoding,
        entries,
    }
}

/// UTF-8 if it is UTF-8, Windows-1252 otherwise.
///
/// `.m3u8` is UTF-8 by definition and `.m3u` frequently is not, but the
/// extension is a claim and the bytes are evidence, so the bytes decide. The
/// order is what makes the fallback safe: valid UTF-8 is never reinterpreted,
/// and accented text in Windows-1252 is not valid UTF-8, so it cannot be
/// mistaken the other way round either.
///
/// Windows-1252 rather than Latin-1 because the files that are not UTF-8 were
/// written by Windows players: the two agree from `0xA0` up, and below it
/// Latin-1 has control codes where Windows-1252 has the typographic
/// punctuation that actually appears in track titles.
fn decode(bytes: &[u8]) -> (String, TextEncoding) {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_owned(), TextEncoding::Utf8),
        Err(_) => (
            bytes.iter().map(|byte| windows_1252_char(*byte)).collect(),
            TextEncoding::Windows1252,
        ),
    }
}

/// The thirty-two code points Windows-1252 puts where Latin-1 keeps C1
/// control codes. Everything else in the range is Latin-1, which is the
/// identity mapping onto Unicode.
fn windows_1252_char(byte: u8) -> char {
    const HIGH: [char; 32] = [
        '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}',
        '\u{017D}', '\u{008F}', '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}',
        '\u{2022}', '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}',
        '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
    ];
    match byte {
        0x80..=0x9F => HIGH[(byte - 0x80) as usize],
        other => char::from(other),
    }
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
mod tests {
    use super::*;

    fn targets(file: &PlaylistFile) -> Vec<String> {
        file.entries
            .iter()
            .map(|entry| entry.segments.join("/"))
            .collect()
    }

    #[test]
    fn a_plain_file_is_one_target_per_line() {
        let file = parse(b"Artist/Album/01.flac\nArtist/Album/02.flac\n");
        assert_eq!(file.encoding, TextEncoding::Utf8);
        assert_eq!(file.name, None);
        assert_eq!(
            targets(&file),
            ["Artist/Album/01.flac", "Artist/Album/02.flac"]
        );
        assert_eq!(file.entries[1].line, 2);
        assert!(file.entries.iter().all(|entry| entry.title.is_none()));
    }

    #[test]
    fn the_extended_dialect_attaches_its_metadata_to_the_next_target() {
        let file = parse(
            b"#EXTM3U\n#PLAYLIST:Evening\n#EXTINF:245,Harbour Bells - Lamp Hymn\nArtist/01.flac\n\
              # a human comment\n#EXTINF:-1,A stream\nArtist/02.flac\n",
        );
        assert_eq!(file.name.as_deref(), Some("Evening"));
        assert_eq!(targets(&file), ["Artist/01.flac", "Artist/02.flac"]);
        assert_eq!(file.entries[0].duration_secs, Some(245));
        assert_eq!(
            file.entries[0].artist_and_title(),
            Some(("Harbour Bells", "Lamp Hymn"))
        );
        // `-1` is a stream's duration, which is not a measurement.
        assert_eq!(file.entries[1].duration_secs, None);
        assert_eq!(file.entries[1].title.as_deref(), Some("A stream"));
        assert_eq!(file.entries[1].artist_and_title(), None);
    }

    #[test]
    fn an_extinf_with_no_target_after_it_carries_nothing_forward() {
        // The metadata belongs to a target. A file that ends on `#EXTINF`
        // must not hand it to whatever the next import reads.
        let file = parse(b"#EXTM3U\n#EXTINF:1,Orphan\n");
        assert!(file.entries.is_empty());
        let file = parse(b"#EXTM3U\n#EXTINF:1,Orphan\n\n#EXTINF:2,Owner - Song\nA/01.flac\n");
        assert_eq!(file.entries.len(), 1);
        assert_eq!(file.entries[0].title.as_deref(), Some("Owner - Song"));
    }

    #[test]
    fn windows_line_endings_and_a_bom_do_not_reach_the_path() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"#EXTM3U\r\nArtist/Album/01.flac\r\n");
        let file = parse(&bytes);
        assert_eq!(file.encoding, TextEncoding::Utf8);
        assert_eq!(targets(&file), ["Artist/Album/01.flac"]);
    }

    #[test]
    fn a_file_that_is_not_utf8_is_read_as_windows_1252() {
        // "Bj<F6>rk/Vespertine/01.flac" — the byte a Windows player writes
        // for ö, which is not valid UTF-8.
        let mut bytes = b"Bj".to_vec();
        bytes.push(0xF6);
        bytes.extend_from_slice(b"rk/Vespertine/01.flac\n");
        let file = parse(&bytes);
        assert_eq!(file.encoding, TextEncoding::Windows1252);
        assert_eq!(targets(&file), ["Björk/Vespertine/01.flac"]);

        // And the thirty-two code points that tell Windows-1252 from Latin-1:
        // 0x92 is a right single quote in a title, not a control code.
        let mut bytes = b"#EXTINF:1,L".to_vec();
        bytes.push(0x92);
        bytes.extend_from_slice(b"artiste - Song\nA/01.flac\n");
        let file = parse(&bytes);
        assert_eq!(
            file.entries[0].title.as_deref(),
            Some("L\u{2019}artiste - Song")
        );
    }

    #[test]
    fn every_shape_of_target_reduces_to_the_same_segments() {
        let expected = ["Artist", "Album", "01.flac"];
        for target in [
            "Artist/Album/01.flac",
            "./Artist/Album/01.flac",
            "../../Artist/Album/01.flac",
            "/home/someone/Music/Artist/Album/01.flac",
            "Artist\\Album\\01.flac",
            "C:\\Users\\someone\\Music\\Artist\\Album\\01.flac",
            "file:///home/someone/Music/Artist/Album/01.flac",
            "file://server/Music/Artist/Album/01.flac",
            // A scheme is case-insensitive, and a player that shouts it names
            // the same file.
            "File:///home/someone/Music/Artist/Album/01.flac",
            "FILE://server/Music/Artist/Album/01.flac",
            "Artist/Album/%30%31.flac",
        ] {
            let file = parse(target.as_bytes());
            let entry = &file.entries[0];
            let tail = entry.segments[entry.segments.len() - 3..].to_vec();
            assert_eq!(tail, expected, "{target}");
            assert_eq!(entry.raw, target, "the report quotes what was written");
        }
    }

    #[test]
    fn a_remote_url_names_no_path_at_all() {
        // Deliberately not a failed path match: a web radio is not a track
        // this catalogue could hold, and the ladder has nothing to climb.
        // And the case-insensitive scheme match does not widen to another
        // scheme that merely looks like one.
        for target in [
            "http://radio.example/stream",
            "https://radio.example/s.mp3",
            "HTTP://radio.example/stream",
            "filesystem://radio.example/s.mp3",
        ] {
            let file = parse(target.as_bytes());
            assert!(file.entries[0].segments.is_empty(), "{target}");
        }
    }

    #[test]
    fn a_hyphen_without_spaces_belongs_to_the_name() {
        let file = parse(b"#EXTINF:1,Jean-Michel Jarre\nA/01.flac\n");
        assert_eq!(file.entries[0].artist_and_title(), None);
        let file = parse(b"#EXTINF:1,Jean-Michel Jarre - Oxygene-4\nA/01.flac\n");
        assert_eq!(
            file.entries[0].artist_and_title(),
            Some(("Jean-Michel Jarre", "Oxygene-4"))
        );
    }
}
