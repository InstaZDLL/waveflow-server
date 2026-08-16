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
    fn rejects_negative_and_overflowing_timestamps() {
        assert_eq!(parse_timestamp("-1:00"), None);
        assert_eq!(parse_timestamp("00:-01"), None);
        assert_eq!(parse_timestamp(&format!("{}:00", i64::MAX)), None);
    }
}
