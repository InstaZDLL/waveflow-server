use super::*;

fn tags(track: (Option<f64>, Option<f64>), album: (Option<f64>, Option<f64>)) -> ReplayGainTags {
    ReplayGainTags {
        track_gain: track.0,
        track_peak: track.1,
        album_gain: album.0,
        album_peak: album.1,
    }
}

#[test]
fn each_mode_reads_its_own_pair_and_falls_back_to_the_other() {
    let both = tags((Some(-6.0), None), (Some(-3.0), None));
    assert_eq!(gain(both, ReplayGainMode::Track).unwrap().decibels(), -6.0);
    assert_eq!(gain(both, ReplayGainMode::Album).unwrap().decibels(), -3.0);
    assert_eq!(gain(both, ReplayGainMode::Off), None);
    let album_only = tags((None, None), (Some(-4.5), None));
    assert_eq!(
        gain(album_only, ReplayGainMode::Track).unwrap().decibels(),
        -4.5
    );
    let track_only = tags((Some(-2.25), None), (None, None));
    assert_eq!(
        gain(track_only, ReplayGainMode::Album).unwrap().decibels(),
        -2.25
    );
    assert_eq!(gain(ReplayGainTags::default(), ReplayGainMode::Album), None);
    assert_eq!(
        gain(
            tags((Some(f64::NAN), None), (None, None)),
            ReplayGainMode::Track
        ),
        None
    );
}

#[test]
fn a_raised_track_never_clips_and_a_wrong_tag_is_bounded() {
    // +6 dB on a peak of 0.8 would clip; it is held at what 0.8 allows.
    let hot = tags((Some(6.0), Some(0.8)), (None, None));
    let held = gain(hot, ReplayGainMode::Track).unwrap().decibels();
    assert!((held - 1.94).abs() < 0.005, "{held}");
    // A cut is never limited by the peak.
    let cut = tags((Some(-8.0), Some(1.0)), (None, None));
    assert_eq!(gain(cut, ReplayGainMode::Track).unwrap().decibels(), -8.0);
    let absurd = tags((Some(90.0), None), (None, None));
    assert_eq!(
        gain(absurd, ReplayGainMode::Track).unwrap().decibels(),
        18.0
    );
    let silent = tags((Some(-500.0), None), (None, None));
    assert_eq!(
        gain(silent, ReplayGainMode::Track).unwrap().decibels(),
        -60.0
    );
    assert_eq!(
        gain(
            tags((Some(0.001), None), (None, None)),
            ReplayGainMode::Track
        ),
        None
    );
}

#[test]
fn modes_are_read_as_written() {
    assert_eq!(ReplayGainMode::parse("Album"), Some(ReplayGainMode::Album));
    assert_eq!(
        ReplayGainMode::parse(" track "),
        Some(ReplayGainMode::Track)
    );
    assert_eq!(ReplayGainMode::parse("off"), Some(ReplayGainMode::Off));
    assert_eq!(ReplayGainMode::parse("loud"), None);
    assert_eq!(Gain { centibels: -650 }.filter(), "volume=-6.50dB");
}
