use super::*;

fn record(json: &str) -> Record {
    serde_json::from_str(json).unwrap()
}

#[test]
fn a_record_is_judged_by_what_it_carries() {
    let hit = judge(
        record(r#"{"duration":234,"plainLyrics":"a","syncedLyrics":"[00:01.00]a"}"#),
        234_400,
    );
    assert_eq!(hit.outcome, LyricsOutcome::Hit);
    assert_eq!(hit.synced.as_deref(), Some("[00:01.00]a"));
    let instrumental = judge(record(r#"{"instrumental":true,"plainLyrics":"x"}"#), 1_000);
    assert_eq!(instrumental.outcome, LyricsOutcome::Instrumental);
    assert!(instrumental.plain.is_none());
    let blank = judge(record(r#"{"plainLyrics":"  ","syncedLyrics":null}"#), 1_000);
    assert_eq!(blank.outcome, LyricsOutcome::Miss);
}

#[test]
fn another_duration_is_another_recording() {
    // Two seconds either way is LRCLIB's own tolerance; three is not.
    let close = judge(record(r#"{"duration":236,"plainLyrics":"a"}"#), 234_000);
    assert_eq!(close.outcome, LyricsOutcome::Hit);
    let far = judge(record(r#"{"duration":237.5,"plainLyrics":"a"}"#), 234_000);
    assert_eq!(far.outcome, LyricsOutcome::Miss);
    // An instrumental is held to the same tolerance: another recording's
    // silence is not this track's answer, and would stand for 30 days.
    let other = judge(record(r#"{"duration":300,"instrumental":true}"#), 234_000);
    assert_eq!(other.outcome, LyricsOutcome::Miss);
    let ours = judge(record(r#"{"duration":235,"instrumental":true}"#), 234_000);
    assert_eq!(ours.outcome, LyricsOutcome::Instrumental);
}

#[test]
fn the_key_folds_spelling_and_refuses_what_cannot_match() {
    let track = |artist: &str, title: &str, duration_ms| TrackLookup {
        artist: artist.into(),
        title: title.into(),
        album: Some("Post".into()),
        duration_ms,
    };
    assert_eq!(
        track("Björk", "Army of Me", 234_000).key(),
        track("BJÖRK", "army of me", 234_200).key()
    );
    assert_ne!(
        track("Björk", "Army of Me", 234_000).key(),
        track("Björk", "Army of Me", 236_000).key()
    );
    assert!(track("", "Army of Me", 234_000).key().is_none());
    assert!(track("Björk", "Army of Me", 0).key().is_none());
}

#[test]
fn the_user_agent_names_the_project() {
    assert!(USER_AGENT.starts_with("WaveFlowServer/"));
    assert!(USER_AGENT.ends_with("(https://github.com/InstaZDLL/waveflow-server)"));
}
