use super::*;

#[test]
fn a_station_address_is_absolute_http_and_nothing_else() {
    assert!(validated_url("http://radio.example.org/stream").is_ok());
    assert!(validated_url("https://radio.example.org:8000/stream.mp3?x=1").is_ok());
    // Trimmed, and the trimming is what is stored.
    assert_eq!(
        validated_url("  https://radio.example.org/s  ").unwrap(),
        "https://radio.example.org/s"
    );
    // Stored as typed: no trailing slash the caller did not write.
    assert_eq!(
        validated_url("https://radio.example.org").unwrap(),
        "https://radio.example.org"
    );

    // Nothing the server could later be made to read off its own disk, and
    // nothing that names no host.
    //
    // **`mms://` is the one that proves the scheme line**, and it is here
    // for that reason: measured by inversion, `file:` and `data:` are
    // already refused for carrying no host, so widening the scheme list to
    // admit them changes no verdict. A scheme with an authority is the only
    // case where the two rules can be told apart.
    assert!(validated_url("mms://radio.example.org/stream").is_err());
    assert!(validated_url("file:///etc/passwd").is_err());
    assert!(validated_url("data:audio/mpeg;base64,AAAA").is_err());
    assert!(validated_url("/stream.mp3").is_err());
    assert!(validated_url("radio.example.org/stream").is_err());
    // An address with no host at all does not reach this function's
    // judgement: the parser refuses it first.
    assert!(validated_url("http://").is_err());
    assert!(validated_url("http://:8000/stream").is_err());
    // And one that looks hostless is not. The WHATWG parser collapses the
    // slashes after a special scheme, so this is a URL whose host is
    // `stream` — surprising enough to be written down rather than
    // discovered by whoever reads the column next.
    assert_eq!(validated_url("http:///stream").unwrap(), "http:///stream");
    assert!(validated_url("").is_err());
    assert!(validated_url(&format!("https://e.org/{}", "a".repeat(2_048))).is_err());
}

#[test]
fn a_blank_homepage_means_none_and_a_blank_address_never_does() {
    assert_eq!(validated_homepage(None).unwrap(), None);
    assert_eq!(validated_homepage(Some("")).unwrap(), None);
    assert_eq!(validated_homepage(Some("   ")).unwrap(), None);
    assert_eq!(
        validated_homepage(Some("https://radio.example.org/")).unwrap(),
        Some("https://radio.example.org/".to_owned())
    );
    // Still judged when it says something.
    assert!(validated_homepage(Some("javascript:alert(1)")).is_err());
    // And the address it sits beside gets no such reading: it is required.
    assert!(validated_url("").is_err());
}

#[test]
fn a_name_is_trimmed_and_cannot_be_blank() {
    assert_eq!(validated_name("  Radio Nova ").unwrap(), "Radio Nova");
    assert!(validated_name("   ").is_err());
    assert!(validated_name("").is_err());
    assert!(validated_name(&"n".repeat(MAX_RADIO_NAME_CHARS)).is_ok());
    assert!(validated_name(&"n".repeat(MAX_RADIO_NAME_CHARS + 1)).is_err());
}
