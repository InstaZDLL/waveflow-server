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
mod tests;
