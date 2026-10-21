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
        assert_eq!(file.encoding, TextEncoding::UTF_8);
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
        assert_eq!(file.encoding, TextEncoding::UTF_8);
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
        assert_eq!(file.encoding.name(), "windows-1252");
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

    /// What Notepad saves as "Unicode", and what some players export: UTF-16
    /// little-endian behind its byte-order mark. Read as Windows-1252, every
    /// other character was a NUL and no path matched anything.
    #[test]
    fn a_utf16_file_is_read_by_its_byte_order_mark() {
        let text = "#EXTM3U\r\n#EXTINF:245,Björk - Hidden Place\r\nBjörk/Vespertine/01.flac\r\n";
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        let file = parse(&bytes);
        assert_eq!(serde_json::to_value(file.encoding).unwrap(), "utf-16le");
        assert_eq!(targets(&file), ["Björk/Vespertine/01.flac"]);
        assert_eq!(
            file.entries[0].artist_and_title(),
            Some(("Björk", "Hidden Place"))
        );
    }

    /// A playlist written on a Japanese Windows, in Shift_JIS. The Windows-1252
    /// fallback read it without failing, which was worse: every name came back
    /// as Latin nonsense that matched no path and no title.
    #[test]
    fn a_file_in_an_east_asian_code_page_is_read_in_it() {
        let text = "#EXTM3U\r\n#PLAYLIST:夏の夜に聴きたい歌\r\n\
                    #EXTINF:245,宇多田ヒカル - 花束を君に\r\n\
                    宇多田ヒカル/ファントーム/01 花束を君に.flac\r\n\
                    #EXTINF:301,椎名林檎 - 丸の内サディスティック\r\n\
                    椎名林檎/無罪モラトリアム/03 丸の内サディスティック.flac\r\n";
        let (bytes, _, unmappable) = encoding_rs::SHIFT_JIS.encode(text);
        assert!(!unmappable, "Shift_JIS holds the sample");
        assert!(std::str::from_utf8(&bytes).is_err());
        let file = parse(&bytes);
        assert_eq!(serde_json::to_value(file.encoding).unwrap(), "shift_jis");
        assert_eq!(file.name.as_deref(), Some("夏の夜に聴きたい歌"));
        assert_eq!(
            targets(&file),
            [
                "宇多田ヒカル/ファントーム/01 花束を君に.flac",
                "椎名林檎/無罪モラトリアム/03 丸の内サディスティック.flac",
            ]
        );
        assert_eq!(
            file.entries[1].artist_and_title(),
            Some(("椎名林檎", "丸の内サディスティック"))
        );
    }

    /// The report names the encoding the core read, not a second guess that
    /// happens to agree today: decoding each sample with the named encoding
    /// gives back the core's text exactly.
    #[test]
    fn the_encoding_named_is_the_one_the_core_read() {
        let samples: [(&str, &'static Encoding); 6] = [
            (
                "#EXTINF:1,Björk - Cocoon\nBjörk/Vespertine/02.flac\n",
                encoding_rs::WINDOWS_1252,
            ),
            (
                "#EXTINF:1,宇多田ヒカル - 花束を君に\n宇多田ヒカル/01 花束を君に.flac\n",
                encoding_rs::SHIFT_JIS,
            ),
            (
                "#EXTINF:1,王菲 - 红豆\n王菲/唱游/03 红豆.flac\n",
                encoding_rs::GB18030,
            ),
            (
                "#EXTINF:1,周杰倫 - 晴天\n周杰倫/葉惠美/01 晴天.flac\n",
                encoding_rs::BIG5,
            ),
            (
                "#EXTINF:1,아이유 - 밤편지\n아이유/팔레트/01 밤편지.flac\n",
                encoding_rs::EUC_KR,
            ),
            (
                "#EXTINF:1,Björk - Cocoon\nBjörk/Vespertine/02.flac\n",
                encoding_rs::UTF_8,
            ),
        ];
        for (text, encoding) in samples {
            let (bytes, _, unmappable) = encoding.encode(text);
            assert!(!unmappable, "{} holds the sample", encoding.name());
            let named = encoding_read_as(&bytes);
            assert_eq!(
                named.decode_without_bom_handling(&bytes).0,
                waveflow_core::text_encoding::decode(&bytes),
                "{} was named {}",
                encoding.name(),
                named.name()
            );
        }
        let mut marked = vec![0xFE, 0xFF];
        marked.extend("Björk/01.flac".encode_utf16().flat_map(u16::to_be_bytes));
        assert_eq!(encoding_read_as(&marked), encoding_rs::UTF_16BE);
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
