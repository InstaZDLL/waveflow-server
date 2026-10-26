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
        // An empty one says nothing, and the body's, nested inside it, wins.
        .find_map(|node| {
            node.attribute(("http://www.w3.org/XML/1998/namespace", "lang"))
                .map(str::trim)
                .filter(|lang| !lang.is_empty())
        })
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
        // A sheet over the expansion budget is shown as the text it is, one
        // line per line of the file, rather than expanded (RFC-023 decision 5).
        if let Some(parsed) = parse_lrc(content).filter(|parsed| !parsed.is_empty()) {
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

/// Whether `content` is a synced LRC sheet: at least one timed line, and a
/// sheet that fits [`MAX_EXPANDED_LRC_BYTES`] once its timestamps are
/// expanded. One that does not is read as the text it is, so it is stored as
/// unsynced and never expanded on a later read either.
pub fn has_lrc_timestamps(content: &str) -> bool {
    let mut timed = false;
    let mut expanded = 0usize;
    for line in normalize_content(content).lines() {
        let (timestamps, value) = timestamps_and_value(line);
        timed |= !timestamps.is_empty();
        expanded = expanded.saturating_add(
            timestamps
                .len()
                .saturating_mul(EXPANDED_LINE_COST.saturating_add(value.len())),
        );
        if expanded > MAX_EXPANDED_LRC_BYTES {
            return false;
        }
    }
    timed
}

/// The most an LRC sheet may grow to once its timestamps are expanded
/// (RFC-023 decision 5), counted as [`EXPANDED_LINE_COST`] per line plus the
/// line's text.
///
/// A line `[00:12][01:30]chorus` is two lines once read, each carrying the
/// text, and nothing bounded how many timestamps one line may carry. A
/// mebibyte of upload — 100,000 timestamps of five bytes before half a
/// mebibyte of text — expanded to some fifty gigabytes, on the upload and on
/// every read of the stored sheet, and the process died of it. A real sheet
/// repeats a chorus a handful of times: eight mebibytes is eight times the
/// largest sheet the server accepts, and no real sheet comes near it.
pub const MAX_EXPANDED_LRC_BYTES: usize = 8 * 1024 * 1024;

/// What one expanded line costs beyond its text: the line itself, so that
/// a sheet of bare timestamps is bounded too.
const EXPANDED_LINE_COST: usize = 32;

/// The timed lines of an LRC sheet, or `None` when expanding its timestamps
/// would outgrow [`MAX_EXPANDED_LRC_BYTES`].
fn parse_lrc(content: &str) -> Option<Vec<LyricsLine>> {
    let mut parsed = Vec::<(i64, usize, String)>::new();
    let mut expanded = 0usize;
    for (order, raw) in normalize_content(content).lines().enumerate() {
        let (timestamps, value) = timestamps_and_value(raw);
        for start in timestamps {
            expanded = expanded
                .saturating_add(EXPANDED_LINE_COST)
                .saturating_add(value.len());
            if expanded > MAX_EXPANDED_LRC_BYTES {
                return None;
            }
            parsed.push((start, order, value.to_owned()));
        }
    }
    parsed.sort_by_key(|(start, order, _)| (*start, *order));
    Some(
        parsed
            .into_iter()
            .map(|(start, _, value)| LyricsLine {
                start: Some(start),
                value,
            })
            .collect(),
    )
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
mod tests;
