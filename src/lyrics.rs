//! Lyrics read models and deterministic LRC parsing shared by native and
//! OpenSubsonic responses.

use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct LyricsInput {
    pub source: &'static str,
    pub lang: String,
    pub synced: bool,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LyricsList {
    pub track_id: Uuid,
    pub structured_lyrics: Vec<StructuredLyrics>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StructuredLyrics {
    pub display_artist: Option<String>,
    pub display_title: String,
    pub lang: String,
    pub synced: bool,
    #[serde(rename = "line")]
    pub lines: Vec<LyricsLine>,
    /// Where this sheet came from: `upload_ttml`, `upload_lrc`, `upload_txt`,
    /// `sidecar_lrc`, `sidecar_text` or `embedded` (RFC-012 decision 6).
    ///
    /// Sent by the native route only. The Subsonic façade builds its sheets
    /// without it, and `None` is never serialised, so its frozen answer keeps
    /// the shape validated clients were tested against.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// The formats a sheet can be uploaded in (RFC-012 decision 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadFormat {
    Lrc,
    Txt,
    Ttml,
}

impl UploadFormat {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "lrc" => Some(Self::Lrc),
            "txt" => Some(Self::Txt),
            "ttml" => Some(Self::Ttml),
            _ => None,
        }
    }

    /// As stored, and as the file extension a download carries.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lrc => "lrc",
            Self::Txt => "txt",
            Self::Ttml => "ttml",
        }
    }

    /// The `source` a sheet read from this upload reports.
    pub fn source(self) -> &'static str {
        match self {
            Self::Lrc => "upload_lrc",
            Self::Txt => "upload_txt",
            Self::Ttml => "upload_ttml",
        }
    }
}

/// An uploaded sheet, read: its language and its lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedUpload {
    pub lang: String,
    pub synced: bool,
    pub lines: Vec<LyricsLine>,
}

/// The language a sheet that names none carries, as OpenSubsonic spells it.
pub const UNKNOWN_LANG: &str = "xxx";

/// Read an uploaded sheet, or `None` when it holds nothing usable.
///
/// "Usable" is the RFC's rule, so an empty upload is refused at the door rather
/// than stored to hide a real sheet later: a `.txt` with no non-blank line, an
/// `.lrc` with no line of text, a `.ttml` from which no timed line parses.
pub fn parse_upload(format: UploadFormat, content: &str) -> Option<ParsedUpload> {
    let parsed = match format {
        UploadFormat::Txt => ParsedUpload {
            lang: UNKNOWN_LANG.to_owned(),
            synced: false,
            lines: lines(content, false),
        },
        UploadFormat::Lrc => {
            let synced = has_lrc_timestamps(content);
            ParsedUpload {
                lang: lrc_language(content).unwrap_or_else(|| UNKNOWN_LANG.to_owned()),
                synced,
                lines: lines(content, synced),
            }
        }
        UploadFormat::Ttml => parse_ttml(content)?,
    };
    has_text(&parsed.lines).then_some(parsed)
}

/// Whether any line says something. A sheet of blank lines is no sheet.
pub fn has_text(lines: &[LyricsLine]) -> bool {
    lines.iter().any(|line| !line.value.trim().is_empty())
}

/// The `[la:…]` tag an LRC file may carry, the only language it can name.
fn lrc_language(content: &str) -> Option<String> {
    normalize_content(content).lines().find_map(|line| {
        let tag = line.trim().strip_prefix("[la:")?.strip_suffix(']')?.trim();
        (!tag.is_empty()).then(|| tag.to_owned())
    })
}

/// The lines of a TTML document: each `<p>` with a `begin` it can read.
///
/// Word timings in `<span>`s are kept in the stored original and not served —
/// both contracts carry line timings only (RFC-012 decision 6) — so a line's
/// text is its words joined, whitespace collapsed. A `<p>` whose `begin` is in
/// frames, or missing, is left out rather than guessed at. The document's
/// `xml:lang`, on the root or the body, names the language.
///
/// `roxmltree`'s default options refuse a DTD, so no entity is ever expanded:
/// an uploaded document cannot reach the file system or inflate itself.
pub fn parse_ttml(content: &str) -> Option<ParsedUpload> {
    let document = roxmltree::Document::parse(content).ok()?;
    let root = document.root_element();
    if root.tag_name().name() != "tt" {
        return None;
    }
    // The root's, else the body's: a `xml:lang` deeper down names one
    // fragment — a span, a paragraph, a head element — not the sheet.
    let lang = std::iter::once(root)
        .chain(
            root.children()
                .filter(|node| node.is_element() && node.tag_name().name() == "body"),
        )
        .find_map(|node| node.attribute(("http://www.w3.org/XML/1998/namespace", "lang")))
        .map(str::trim)
        .filter(|lang| !lang.is_empty())
        .unwrap_or(UNKNOWN_LANG)
        .to_owned();
    // Outermost paragraphs only, found without walking into one: a `<p>`
    // nested in a `<p>` is not TTML, and reading each paragraph's whole
    // subtree would copy the text once per level of nesting — quadratic in a
    // document the size cap alone does not bound. Each text node is read once.
    let mut paragraphs = Vec::new();
    let mut pending: Vec<_> = root.children().rev().collect();
    while let Some(node) = pending.pop() {
        if node.is_element() && node.tag_name().name() == "p" {
            paragraphs.push(node);
        } else {
            pending.extend(node.children().rev());
        }
    }
    let mut lines = Vec::new();
    for (order, paragraph) in paragraphs.into_iter().enumerate() {
        let Some(start) = paragraph.attribute("begin").and_then(ttml_time) else {
            continue;
        };
        // A `<br/>` breaks the line on screen; here it keeps the words on
        // either side of it apart.
        let text = paragraph
            .descendants()
            .filter_map(|node| {
                if node.is_text() {
                    node.text()
                } else if node.is_element() && node.tag_name().name() == "br" {
                    Some(" ")
                } else {
                    None
                }
            })
            .collect::<String>();
        let value = text.split_whitespace().collect::<Vec<_>>().join(" ");
        lines.push((start, order, value));
    }
    lines.sort_by_key(|(start, order, _)| (*start, *order));
    Some(ParsedUpload {
        lang,
        synced: true,
        lines: lines
            .into_iter()
            .map(|(start, _, value)| LyricsLine {
                start: Some(start),
                value,
            })
            .collect(),
    })
}

/// A TTML time expression in milliseconds.
///
/// Clock time — `hh:mm:ss`, `mm:ss` or `ss`, each with an optional fraction
/// — and offset time with a metric: `h`, `m`, `s` or `ms`. Frames (a fourth
/// clock field, or the `f` and `t` metrics) need the document's frame and tick
/// rates, which a lyrics sheet has no business making the server compute; they
/// answer `None`, and the line is left out.
fn ttml_time(value: &str) -> Option<i64> {
    let value = value.trim();
    let offset = |number: &str, unit_ms: f64| -> Option<i64> {
        let number: f64 = number.parse().ok()?;
        let ms = number * unit_ms;
        (ms.is_finite() && ms >= 0.0 && ms <= i64::MAX as f64).then(|| ms.round() as i64)
    };
    if let Some(number) = value.strip_suffix("ms") {
        return offset(number, 1.0);
    }
    if let Some(number) = value.strip_suffix('h') {
        return offset(number, 3_600_000.0);
    }
    if let Some(number) = value.strip_suffix('m') {
        return offset(number, 60_000.0);
    }
    if let Some(number) = value.strip_suffix('s') {
        return offset(number, 1_000.0);
    }
    let fields: Vec<&str> = value.split(':').collect();
    if fields.is_empty() || fields.len() > 3 || fields.iter().any(|field| field.is_empty()) {
        return None;
    }
    let (whole, last) = fields.split_at(fields.len() - 1);
    let seconds: f64 = last[0].parse().ok()?;
    if !seconds.is_finite() || seconds < 0.0 || (!whole.is_empty() && seconds >= 60.0) {
        return None;
    }
    let mut total = 0i64;
    for (index, field) in whole.iter().enumerate() {
        let number: i64 = field.parse().ok()?;
        // Hours are unbounded; minutes under hours are not.
        if number < 0 || (index == whole.len() - 1 && whole.len() == 2 && number >= 60) {
            return None;
        }
        let unit = if whole.len() - index == 2 {
            3_600_000
        } else {
            60_000
        };
        total = total.checked_add(number.checked_mul(unit)?)?;
    }
    total.checked_add((seconds * 1000.0).round() as i64)
}

/// A synced sheet written back as LRC, for a download of lyrics the scanner
/// found: `[mm:ss.cc]` per line, minutes unbounded as LRC allows.
pub fn to_lrc(lines: &[LyricsLine]) -> String {
    let mut out = String::new();
    for line in lines {
        if let Some(start) = line.start {
            let centis = (start % 1000) / 10;
            let seconds = (start / 1000) % 60;
            let minutes = start / 60_000;
            out.push_str(&format!("[{minutes:02}:{seconds:02}.{centis:02}]"));
        }
        out.push_str(&line.value);
        out.push('\n');
    }
    out
}

/// A plain sheet written back as text, one line per line.
pub fn to_text(lines: &[LyricsLine]) -> String {
    let mut out = String::new();
    for line in lines {
        out.push_str(&line.value);
        out.push('\n');
    }
    out
}

#[derive(Debug, Clone, Serialize, ToSchema, PartialEq, Eq)]
pub struct LyricsLine {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<i64>,
    pub value: String,
}

pub fn normalize_content(content: &str) -> String {
    content.replace("\r\n", "\n").replace('\r', "\n")
}

pub fn lines(content: &str, synced: bool) -> Vec<LyricsLine> {
    if synced {
        let parsed = parse_lrc(content);
        if !parsed.is_empty() {
            return parsed;
        }
    }
    normalize_content(content)
        .lines()
        .map(|value| LyricsLine {
            start: None,
            value: value.to_owned(),
        })
        .collect()
}

pub fn has_lrc_timestamps(content: &str) -> bool {
    normalize_content(content)
        .lines()
        .any(|line| timestamps_and_value(line).0.into_iter().next().is_some())
}

fn parse_lrc(content: &str) -> Vec<LyricsLine> {
    let mut parsed = Vec::<(i64, usize, String)>::new();
    for (order, raw) in normalize_content(content).lines().enumerate() {
        let (timestamps, value) = timestamps_and_value(raw);
        for start in timestamps {
            parsed.push((start, order, value.to_owned()));
        }
    }
    parsed.sort_by_key(|(start, order, _)| (*start, *order));
    parsed
        .into_iter()
        .map(|(start, _, value)| LyricsLine {
            start: Some(start),
            value,
        })
        .collect()
}

fn timestamps_and_value(mut input: &str) -> (Vec<i64>, &str) {
    let mut timestamps = Vec::new();
    while let Some(rest) = input.strip_prefix('[') {
        let Some(end) = rest.find(']') else { break };
        let token = &rest[..end];
        let Some(timestamp) = parse_timestamp(token) else {
            break;
        };
        timestamps.push(timestamp);
        input = &rest[end + 1..];
    }
    (timestamps, input)
}

fn parse_timestamp(value: &str) -> Option<i64> {
    let (minutes, seconds) = value.split_once(':')?;
    if minutes.is_empty() || seconds.is_empty() {
        return None;
    }
    let minutes = minutes.parse::<i64>().ok()?;
    let (seconds, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
    let seconds = seconds.parse::<i64>().ok()?;
    if minutes < 0
        || !(0..60).contains(&seconds)
        || fraction.len() > 3
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let millis = match fraction.len() {
        0 => 0,
        1 => fraction.parse::<i64>().ok()? * 100,
        2 => fraction.parse::<i64>().ok()? * 10,
        3 => fraction.parse::<i64>().ok()?,
        _ => return None,
    };
    minutes
        .checked_mul(60_000)?
        .checked_add(seconds.checked_mul(1_000)?)?
        .checked_add(millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ttml_line_break_keeps_its_words_apart() {
        let parsed = parse_ttml(
            r#"<tt xmlns="http://www.w3.org/ns/ttml"><body><div>
                <p begin="1s">hello<br/>world</p>
            </div></body></tt>"#,
        )
        .unwrap();
        assert_eq!(parsed.lines[0].value, "hello world");
    }

    #[test]
    fn a_ttml_language_comes_from_the_root_or_the_body_only() {
        let deep = parse_ttml(
            r#"<tt xmlns="http://www.w3.org/ns/ttml"><head><metadata xml:lang="de"/></head>
                <body><div><p begin="1s"><span xml:lang="en">hi</span></p></div></body></tt>"#,
        )
        .unwrap();
        assert_eq!(deep.lang, UNKNOWN_LANG);
        let body = parse_ttml(
            r#"<tt xmlns="http://www.w3.org/ns/ttml"><head><metadata xml:lang="de"/></head>
                <body xml:lang="ko"><div><p begin="1s">hi</p></div></body></tt>"#,
        )
        .unwrap();
        assert_eq!(body.lang, "ko");
    }

    #[test]
    fn a_paragraph_nested_in_a_paragraph_reads_once() {
        let parsed = parse_ttml(
            r#"<tt xmlns="http://www.w3.org/ns/ttml"><body><div>
                <p begin="1s">outer <p begin="2s">inner <p begin="3s">deepest</p></p></p>
                <p begin="4s">next</p>
            </div></body></tt>"#,
        )
        .unwrap();
        assert_eq!(
            parsed.lines,
            vec![
                LyricsLine {
                    start: Some(1000),
                    value: "outer inner deepest".into(),
                },
                LyricsLine {
                    start: Some(4000),
                    value: "next".into(),
                },
            ]
        );
    }

    #[test]
    fn parses_and_orders_lrc_timestamps() {
        assert_eq!(
            lines("[00:02.50]second\n[00:01.125][00:03]shared", true),
            vec![
                LyricsLine {
                    start: Some(1125),
                    value: "shared".into(),
                },
                LyricsLine {
                    start: Some(2500),
                    value: "second".into(),
                },
                LyricsLine {
                    start: Some(3000),
                    value: "shared".into(),
                },
            ]
        );
    }

    #[test]
    fn ignores_lrc_metadata_and_preserves_plain_lines() {
        assert_eq!(
            lines("[ar:WaveFlow]\r\nfirst\r\n\r\nsecond", false),
            vec![
                LyricsLine {
                    start: None,
                    value: "[ar:WaveFlow]".into(),
                },
                LyricsLine {
                    start: None,
                    value: "first".into(),
                },
                LyricsLine {
                    start: None,
                    value: "".into(),
                },
                LyricsLine {
                    start: None,
                    value: "second".into(),
                },
            ]
        );
    }

    #[test]
    fn reads_every_ttml_clock_and_offset_form() {
        assert_eq!(ttml_time("01:02:03.5"), Some(3_723_500));
        assert_eq!(ttml_time("2:03.250"), Some(123_250));
        assert_eq!(ttml_time("12.345"), Some(12_345));
        assert_eq!(ttml_time("1.5s"), Some(1_500));
        assert_eq!(ttml_time("1500ms"), Some(1_500));
        assert_eq!(ttml_time("2m"), Some(120_000));
        assert_eq!(ttml_time("1h"), Some(3_600_000));
        // Frames need a frame rate the sheet does not get to set.
        assert_eq!(ttml_time("00:00:01:12"), None);
        assert_eq!(ttml_time("12f"), None);
        // Out of range, and not a time at all.
        assert_eq!(ttml_time("00:61:00"), None);
        assert_eq!(ttml_time("-1s"), None);
        assert_eq!(ttml_time(""), None);
    }

    #[test]
    fn reads_a_ttml_sheet_with_word_spans() {
        let ttml = r#"<?xml version="1.0" encoding="UTF-8"?>
<tt xmlns="http://www.w3.org/ns/ttml" xml:lang="ja">
  <body><div>
    <p begin="0:05.500" end="0:07"><span begin="0:05.500">夜の</span> <span begin="0:06">街</span></p>
    <p begin="0:01.000">first</p>
    <p>no time, left out</p>
    <p begin="00:00:09:05">frames, left out</p>
  </div></body>
</tt>"#;
        let parsed = parse_ttml(ttml).expect("a sheet");
        assert_eq!(parsed.lang, "ja");
        assert!(parsed.synced);
        assert_eq!(
            parsed.lines,
            vec![
                LyricsLine {
                    start: Some(1_000),
                    value: "first".into(),
                },
                LyricsLine {
                    start: Some(5_500),
                    value: "夜の 街".into(),
                },
            ]
        );
    }

    #[test]
    fn refuses_a_ttml_document_with_a_dtd() {
        // An entity is how an XML upload reaches a file or inflates itself;
        // the default parser options refuse the DTD that would declare one.
        let hostile = r#"<?xml version="1.0"?>
<!DOCTYPE tt [<!ENTITY x SYSTEM "file:///etc/passwd">]>
<tt xmlns="http://www.w3.org/ns/ttml"><body><div><p begin="1s">&x;</p></div></body></tt>"#;
        assert_eq!(parse_ttml(hostile), None);
        assert_eq!(parse_upload(UploadFormat::Ttml, hostile), None);
    }

    #[test]
    fn refuses_uploads_with_nothing_usable_in_them() {
        assert_eq!(parse_upload(UploadFormat::Txt, " \n\t\n"), None);
        assert_eq!(
            parse_upload(UploadFormat::Lrc, "[ar:Someone]\n[00:01.00]\n"),
            None
        );
        assert_eq!(
            parse_upload(
                UploadFormat::Ttml,
                r#"<tt xmlns="http://www.w3.org/ns/ttml"><body><div><p>untimed</p></div></body></tt>"#
            ),
            None
        );
        assert_eq!(
            parse_upload(UploadFormat::Ttml, "<html><p begin=\"1s\">x</p></html>"),
            None
        );
        let lrc = parse_upload(UploadFormat::Lrc, "[la:fr]\n[00:01.00]bonjour").expect("a sheet");
        assert_eq!(lrc.lang, "fr");
        assert!(lrc.synced);
        let plain = parse_upload(UploadFormat::Lrc, "no timestamps\nat all").expect("a sheet");
        assert!(!plain.synced, "an .lrc without timings is plain text");
    }

    #[test]
    fn writes_scanned_sheets_back_as_lrc_and_text() {
        let synced = lines("[00:01.25]one\n[61:02.5]two", true);
        assert_eq!(to_lrc(&synced), "[00:01.25]one\n[61:02.50]two\n");
        assert_eq!(lines(&to_lrc(&synced), true), synced, "round trip");
        assert_eq!(to_text(&lines("a\n\nb", false)), "a\n\nb\n");
    }

    #[test]
    fn rejects_negative_and_overflowing_timestamps() {
        assert_eq!(parse_timestamp("-1:00"), None);
        assert_eq!(parse_timestamp("00:-01"), None);
        assert_eq!(parse_timestamp(&format!("{}:00", i64::MAX)), None);
    }
}
