use super::*;

const OPUS: Option<(OutputFormat, u32)> = Some((OutputFormat::Opus, 128));

#[test]
fn a_source_above_the_ceiling_is_encoded() {
    assert_eq!(profile_outcome("opus", Some(128), Some(129)), OPUS);
}

#[test]
fn a_source_at_or_below_the_ceiling_is_left_alone() {
    assert_eq!(profile_outcome("opus", Some(128), Some(128)), None);
    assert_eq!(profile_outcome("opus", Some(128), Some(96)), None);
}

#[test]
fn a_source_of_unknown_bit_rate_is_encoded() {
    assert_eq!(profile_outcome("opus", Some(128), None), OPUS);
}

#[test]
fn an_override_of_raw_is_no_encode() {
    assert_eq!(profile_outcome("raw", None, Some(1_000)), None);
}

#[test]
fn a_format_this_binary_cannot_read_is_no_profile() {
    assert_eq!(profile_outcome("wma", Some(128), Some(1_000)), None);
}

#[test]
fn a_client_key_is_trimmed_and_a_blank_one_refused() {
    assert_eq!(client_key(None).unwrap(), "");
    assert_eq!(client_key(Some("  Symfonium ")).unwrap(), "Symfonium");
    assert!(matches!(
        client_key(Some("   ")),
        Err(ServiceError::Invalid)
    ));
    let long = "x".repeat(MAX_CLIENT_CHARS + 1);
    assert!(matches!(
        client_key(Some(&long)),
        Err(ServiceError::Invalid)
    ));
}

#[test]
fn the_default_cannot_be_raw_and_raw_takes_no_bit_rate() {
    assert!(stored_bitrate("", OutputFormat::Raw, None).is_err());
    assert!(stored_bitrate("Feishin", OutputFormat::Raw, Some(128)).is_err());
    assert_eq!(
        stored_bitrate("Feishin", OutputFormat::Raw, None).unwrap(),
        None
    );
}

#[test]
fn a_bit_rate_left_out_takes_the_formats_default_and_is_bounded() {
    assert_eq!(
        stored_bitrate("", OutputFormat::Opus, None).unwrap(),
        Some(128)
    );
    assert!(stored_bitrate("", OutputFormat::Opus, Some(31)).is_err());
    assert!(stored_bitrate("", OutputFormat::Opus, Some(513)).is_err());
}

/// The bound drops the name heard from longest ago, and hearing a known name
/// again counts as hearing it now, whatever case it comes in.
#[test]
fn the_seen_names_are_bounded_and_the_latest_kept() {
    let seen = SeenClients::default();
    let user = Uuid::new_v4();
    for index in 0..MAX_SEEN_CLIENTS {
        seen.note(user, &format!("client-{index}"));
    }
    seen.note(user, "CLIENT-0");
    seen.note(user, "newcomer");

    let names = seen.of(user);
    assert_eq!(names.len(), MAX_SEEN_CLIENTS);
    assert_eq!(names.first().unwrap(), "client-2");
    assert_eq!(names[MAX_SEEN_CLIENTS - 2], "CLIENT-0");
    assert_eq!(names.last().unwrap(), "newcomer");
    assert!(seen.of(Uuid::new_v4()).is_empty());
}
