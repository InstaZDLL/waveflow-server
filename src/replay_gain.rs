//! ReplayGain applied by the server, on a Subsonic transcode (RFC-022, what
//! was still open).
//!
//! The web player applies ReplayGain itself, through Web Audio
//! (`webapp/src/lib/replay-gain.ts`), and so do most Subsonic clients, from the
//! `replayGain` element. Some cannot — a car, a smart speaker, a client that
//! ignores the tags — and for those the only place left to level the music is
//! the encode. So a Subsonic `stream` that is transcoded anyway may carry the
//! gain into FFmpeg's `volume` filter; the original file is never altered, and
//! a request for the original is never transcoded to apply it.
//!
//! The rules are the web player's, so a track sounds the same either way:
//! album falls back to track and track to album, an untagged track plays at
//! unity, the gain never raises a peak past full scale, and the total is held
//! to [−60, +18] dB against a tag that is simply wrong. There is no preamp: a
//! server-wide one would be a loudness preference made for every listener.

/// Which gain a transcode applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReplayGainMode {
    #[default]
    Off,
    Track,
    Album,
}

impl ReplayGainMode {
    /// The spellings `WAVEFLOW_SUBSONIC_REPLAYGAIN` and the `replayGain`
    /// stream parameter accept, without regard to case.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "false" | "0" => Some(Self::Off),
            "track" => Some(Self::Track),
            "album" => Some(Self::Album),
            _ => None,
        }
    }
}

/// The four values the scanner stores for a track.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ReplayGainTags {
    pub track_gain: Option<f64>,
    pub track_peak: Option<f64>,
    pub album_gain: Option<f64>,
    pub album_peak: Option<f64>,
}

/// A gain FFmpeg applies, in hundredths of a decibel: rounded, so that two
/// requests for the same track and mode share one cache entry, and an integer,
/// so it can be part of the cache key exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Gain {
    centibels: i32,
}

impl Gain {
    pub fn decibels(self) -> f64 {
        f64::from(self.centibels) / 100.0
    }

    /// FFmpeg's `volume` filter for this gain.
    pub fn filter(self) -> String {
        format!("volume={:.2}dB", self.decibels())
    }

    /// What the cache key names of this gain.
    pub fn key(self) -> String {
        format!("rg{}", self.centibels)
    }
}

const TOTAL_MIN_DB: f64 = -60.0;
const TOTAL_MAX_DB: f64 = 18.0;

/// The gain to apply, or `None` for none at all: the mode is off, or the track
/// carries neither gain, or the result rounds to nothing. `None` keeps the
/// encode, and its cache entry, exactly what it was without ReplayGain.
pub fn gain(tags: ReplayGainTags, mode: ReplayGainMode) -> Option<Gain> {
    let track = (tags.track_gain, tags.track_peak);
    let album = (tags.album_gain, tags.album_peak);
    let (first, second) = match mode {
        ReplayGainMode::Off => return None,
        ReplayGainMode::Track => (track, album),
        ReplayGainMode::Album => (album, track),
    };
    let finite = |value: Option<f64>| value.filter(|value| value.is_finite());
    let (gain, peak) = match (finite(first.0), finite(second.0)) {
        (Some(gain), _) => (gain, first.1),
        (None, Some(gain)) => (gain, second.1),
        (None, None) => return None,
    };
    let mut decibels = gain.clamp(TOTAL_MIN_DB, TOTAL_MAX_DB);
    // A peak is a linear sample amplitude: the gain may raise it to full
    // scale and no further.
    if let Some(peak) = finite(peak).filter(|peak| *peak > 0.0) {
        decibels = decibels.min(-20.0 * peak.log10());
    }
    let centibels = (decibels * 100.0).round() as i32;
    (centibels != 0).then_some(Gain { centibels })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(
        track: (Option<f64>, Option<f64>),
        album: (Option<f64>, Option<f64>),
    ) -> ReplayGainTags {
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
}
