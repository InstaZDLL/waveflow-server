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
    let blank_root = parse_ttml(
        r#"<tt xmlns="http://www.w3.org/ns/ttml" xml:lang=" ">
                <body xml:lang="ko"><div><p begin="1s">hi</p></div></body></tt>"#,
    )
    .unwrap();
    assert_eq!(blank_root.lang, "ko");
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

/// A line may carry several timestamps, and each is a line once read; a
/// sheet that piles them up is read as the text it is instead of being
/// expanded without end (RFC-023 decision 5).
///
/// Sized to be told apart cheaply: twenty thousand timestamps before a
/// kilobyte of text is 120 KB of file and twenty megabytes expanded, so a
/// parser without the budget passes neither assertion and allocates
/// nothing a test run would notice.
#[test]
fn a_sheet_that_piles_up_timestamps_is_not_expanded() {
    let sheet = format!("{}{}", "[0:0]".repeat(20_000), "la".repeat(512));
    assert!(!has_lrc_timestamps(&sheet), "over the budget is not synced");
    assert_eq!(lines(&sheet, true).len(), 1, "read as the one line it is");
    let upload = parse_upload(UploadFormat::Lrc, &sheet).expect("still a sheet");
    assert!(!upload.synced);
    assert_eq!(upload.lines.len(), 1);

    // A chorus repeated, as real sheets do, is still expanded.
    let chorus = lines(
        "[00:10.00][01:10.00][02:10.00]chorus\n[00:20.00]verse",
        true,
    );
    assert_eq!(chorus.len(), 4);
    assert_eq!(chorus[1].value, "verse");
}
