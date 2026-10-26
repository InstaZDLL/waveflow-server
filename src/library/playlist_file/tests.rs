use super::*;

/// The parser with no ceiling: what these tests are about is the reading.
fn parse(bytes: &[u8]) -> PlaylistFile {
    super::parse(bytes, usize::MAX).expect("no ceiling to reach")
}

/// The ceiling is noticed at the first entry past it, and a file exactly
/// at it is read (RFC-023 decision 5). Comments and directives are not
/// entries and count for nothing.
#[test]
fn a_file_past_the_ceiling_is_refused_and_one_at_it_is_read() {
    let at = "#EXTM3U\n#EXTINF:1,A - B\na.flac\nb.flac\nc.flac\n";
    assert_eq!(super::parse(at.as_bytes(), 3).unwrap().entries.len(), 3);
    let past = format!("{at}d.flac\n");
    assert_eq!(super::parse(past.as_bytes(), 3).err(), Some(TooManyEntries));
}

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
