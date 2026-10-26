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

/// A secure address may move anywhere secure, a clear one anywhere; a
/// secure one may not move into the clear. A scheme-relative target
/// (`//host/path`) keeps the scheme it came from, so it never downgrades.
#[test]
fn a_redirect_never_leaves_https_for_http() {
    let secure = url::Url::parse("https://feeds.example.org/show.xml?auth=t").unwrap();
    let clear = url::Url::parse("http://feeds.example.org/show.xml").unwrap();
    assert_eq!(
        next_hop(&secure, "https://cdn.example.org/show.xml")
            .unwrap()
            .as_str(),
        "https://cdn.example.org/show.xml"
    );
    assert_eq!(
        next_hop(&secure, "/moved.xml").unwrap().as_str(),
        "https://feeds.example.org/moved.xml"
    );
    assert_eq!(
        next_hop(&secure, "//cdn.example.org/show.xml")
            .unwrap()
            .as_str(),
        "https://cdn.example.org/show.xml"
    );
    assert_eq!(
        next_hop(&clear, "//cdn.example.org/show.xml")
            .unwrap()
            .as_str(),
        "http://cdn.example.org/show.xml"
    );
    assert_eq!(
        next_hop(&clear, "https://cdn.example.org/show.xml")
            .unwrap()
            .as_str(),
        "https://cdn.example.org/show.xml"
    );
    assert_eq!(
        next_hop(&clear, "http://cdn.example.org/show.xml")
            .unwrap()
            .as_str(),
        "http://cdn.example.org/show.xml"
    );
    assert_eq!(
        next_hop(&secure, "http://cdn.example.org/show.xml"),
        Err(FetchError::Destination)
    );
    assert_eq!(
        next_hop(&secure, "HTTP://cdn.example.org/show.xml"),
        Err(FetchError::Destination)
    );
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
