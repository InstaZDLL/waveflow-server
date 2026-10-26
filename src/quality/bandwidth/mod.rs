//! Where a file's high frequencies stop, and whether they stop like a
//! lossy encoder makes them stop.
//!
//! **A port of `waveflow_core::analysis::bandwidth`** at `BANDWIDTH_VERSION`
//! 1 (RFC-016 decision 2). The core keeps its meter `pub(crate)`, and the
//! server feeds it from FFmpeg rather than from symphonia, so the averaging and
//! the search are carried here unchanged; what is added is
//! [`BandwidthMeter::finish_with_spectrum`], which hands back the averaged
//! spectrum the meter already computed. Two tests hold the port to the
//! original: the public constants must be equal, and `tests/quality.rs` measures
//! the same signals through both and compares.
//!
//! Every lossy encoder low-passes before it encodes: LAME cuts a 128 kbps
//! MP3 around 16-17 kHz and a 320 kbps one around 20 kHz, AAC and Vorbis
//! do the same by bitrate, and Opus never codes past 20 kHz. Decoding the
//! result back into FLAC or WAV keeps the cut. A recording made lossless
//! has no such cut: its spectrum fades into the noise floor, or runs on
//! to the converter's own filter just under Nyquist.
//!
//! So the measure is the spectrum averaged over the whole file, at the
//! native rate and channel by channel (a mono sum cancels highs that sit
//! in opposition between the channels, and would draw a wall a genuine
//! file does not have), and the search is for a **wall**: a fall of tens of dB
//! within a few hundred hertz, with nothing coming back above it. A slow
//! fade, however low it ends, is not a wall — which is what keeps an old,
//! naturally dull recording from being taken for an MP3.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

/// Samples per FFT frame: 10.8 Hz per bin at 44.1 kHz, fine enough to
/// place a wall within a band of a few hundred hertz.
const FRAME: usize = 4096;
/// Frames under this RMS (dBFS) are left out: silence and fades would
/// pull the average towards the noise floor everywhere at once.
const SILENT_DBFS: f64 = -60.0;
/// The band the spectrum is smoothed over before the search, in hertz.
const SMOOTH_HZ: f64 = 100.0;
/// How far on each side of a candidate the levels are compared, in hertz,
/// and the gap left around the candidate itself.
const SPAN_HZ: f64 = 800.0;
const GAP_HZ: f64 = 150.0;
/// The lowest frequency a wall is looked for at. Below this a fall is
/// the music's own (a low-passed mix, a telephone sound), not a codec's.
const SEARCH_FROM_HZ: f64 = 11_000.0;
/// A fall at least this steep across the candidate is a wall.
///
/// Set with the settings below on averaged spectra (`analysis_bench`
/// plus a sweep): 41 genuine WAVs and 40 full-band FLACs, 240 fakes made
/// from those FLACs through LAME, Opus, Vorbis and ffmpeg's AAC, and 600
/// real MP3 and M4A files. At 20 dB no genuine file is flagged; at 15 one
/// WAV was. Dark sources (lo-fi, sparse acoustic) are what a higher
/// setting loses: their highs sit so near the floor that the encoder's
/// cut is a fall of 12-20 dB.
pub const WALL_DB: f64 = 20.0;
/// Above a wall nothing may come back to within this of the level
/// below it. Measured relative to the music, not to the floor: an MP3
/// decoded at 48 kHz leaves faint tones near Nyquist, 60 dB under the
/// music yet well over the floor, and those are not content coming back.
const COMES_BACK_DB: f64 = 15.0;

/// A wall at or under this, in a lossless file, is the mark of a lossy
/// source. LAME's highest setting cuts at 20-20.5 kHz and Opus at 20 kHz,
/// while a recording made lossless runs past 21 kHz or fades with no
/// wall at all.
pub const LOSSY_CUTOFF_HZ: f64 = 21_000.0;

/// How far under Nyquist a wall must stand to count. A converter's own
/// anti-alias filter sits just under it — near 15 kHz for a 32 kHz file,
/// which the fixed threshold above would take for a 128 kbps MP3 — and a
/// 44.1 kHz file is not affected (21.05 kHz is past the threshold).
pub const NYQUIST_MARGIN_HZ: f64 = 1_000.0;

/// The core's generation this port follows. A change to the core's detector
/// bumps its own `BANDWIDTH_VERSION`, and `the_port_follows_the_core` then
/// fails until this module has been brought level and this number with it;
/// following it is a reason to bump `quality::QUALITY_VERSION`.
pub const BANDWIDTH_VERSION: i64 = 1;

/// What [`BandwidthMeter::finish`] measured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bandwidth {
    /// Where the wall stands, in hertz, or `None` when the spectrum has
    /// none — it fades out, or runs to Nyquist.
    pub cutoff_hz: Option<f64>,
    /// How far the level falls across the steepest candidate, in dB.
    /// Kept for tuning; the wall is what `cutoff_hz` reports.
    pub wall_db: f64,
    /// Half the sample rate: the most a file could hold.
    pub nyquist_hz: f64,
}

/// Streaming average power spectrum at the native rate, the channels'
/// powers added.
pub struct BandwidthMeter {
    rate: f64,
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    /// One frame buffer per channel.
    frames_in: Vec<Vec<f32>>,
    filled: usize,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    power: Vec<f64>,
    frames: u64,
}

impl BandwidthMeter {
    pub fn new(rate: f64, channels: usize) -> Self {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(FRAME);
        let spectrum = fft.make_output_vec();
        let scratch = fft.make_scratch_vec();
        let window = (0..FRAME)
            .map(|i| {
                let phase = 2.0 * std::f32::consts::PI * i as f32 / FRAME as f32;
                0.5 - 0.5 * phase.cos()
            })
            .collect();
        Self {
            rate,
            power: vec![0.0; spectrum.len()],
            fft,
            window,
            frames_in: vec![vec![0.0; FRAME]; channels.max(1)],
            filled: 0,
            input: vec![0.0; FRAME],
            spectrum,
            scratch,
            frames: 0,
        }
    }

    /// One sample per channel, in channel order. Extra samples beyond the
    /// channel count given to [`Self::new`] are ignored.
    pub fn push_frame(&mut self, frame: &[f32]) {
        for (buffer, &sample) in self.frames_in.iter_mut().zip(frame) {
            buffer[self.filled] = sample;
        }
        self.filled += 1;
        if self.filled == FRAME {
            self.process();
            self.filled = 0;
        }
    }

    fn process(&mut self) {
        let channels = self.frames_in.len() as f64;
        let energy: f64 = self
            .frames_in
            .iter()
            .flatten()
            .map(|&s| f64::from(s) * f64::from(s))
            .sum();
        let rms_db = 10.0 * (energy / (FRAME as f64 * channels)).max(1e-20).log10();
        if rms_db < SILENT_DBFS {
            return;
        }
        for channel in 0..self.frames_in.len() {
            for ((dst, &src), &w) in self
                .input
                .iter_mut()
                .zip(&self.frames_in[channel])
                .zip(&self.window)
            {
                *dst = src * w;
            }
            if self
                .fft
                .process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch)
                .is_err()
            {
                return;
            }
            for (acc, bin) in self.power.iter_mut().zip(&self.spectrum) {
                *acc += f64::from(bin.norm_sqr());
            }
        }
        self.frames += 1;
    }

    pub fn finish(self) -> Option<Bandwidth> {
        self.finish_with_spectrum().map(|(bandwidth, _)| bandwidth)
    }

    /// [`Self::finish`], and the averaged spectrum it searched: one level in
    /// dB per FFT bin, from 0 Hz to Nyquist. `None` when every frame was
    /// silent.
    pub fn finish_with_spectrum(self) -> Option<(Bandwidth, Vec<f64>)> {
        if self.frames == 0 {
            return None;
        }
        let db: Vec<f64> = self
            .power
            .iter()
            .map(|&p| 10.0 * (p / self.frames as f64).max(1e-20).log10())
            .collect();
        Some((find_wall(&db, self.rate), db))
    }
}

/// The wall in an averaged spectrum (`db[k]` for bin `k` of a
/// [`FRAME`]-point FFT at `rate`), or none.
fn find_wall(db: &[f64], rate: f64) -> Bandwidth {
    let nyquist = rate / 2.0;
    let bin_hz = rate / FRAME as f64;
    let to_bin = |hz: f64| ((hz / bin_hz).round() as usize).min(db.len() - 1);

    // Smoothed over SMOOTH_HZ, so one tonal peak or one notch is not a wall.
    let half = ((SMOOTH_HZ / bin_hz / 2.0).round() as usize).max(1);
    let mut prefix = Vec::with_capacity(db.len() + 1);
    prefix.push(0.0);
    for &v in db {
        prefix.push(prefix.last().unwrap() + v);
    }
    let mean = |lo: usize, hi: usize| -> f64 {
        let (lo, hi) = (lo.min(db.len()), hi.min(db.len()));
        if hi <= lo {
            return f64::NEG_INFINITY;
        }
        (prefix[hi] - prefix[lo]) / (hi - lo) as f64
    };
    let smooth: Vec<f64> = (0..db.len())
        .map(|k| mean(k.saturating_sub(half), k + half + 1))
        .collect();

    let span = to_bin(SPAN_HZ);
    let gap = to_bin(GAP_HZ).max(1);
    let top = to_bin(nyquist - 100.0);
    let first = to_bin(SEARCH_FROM_HZ);
    let last = top.saturating_sub(gap + span / 2);

    let mut best: Option<(usize, f64)> = None;
    for k in first..last {
        let below = mean(k.saturating_sub(gap + span), k - gap);
        let above = mean(k + gap, (k + gap + span).min(top));
        let drop = below - above;
        // Spelled without `Option::is_none_or`: the MSRV is 1.80.
        let better = match best {
            Some((_, d)) => drop > d,
            None => true,
        };
        if better {
            best = Some((k, drop));
        }
    }
    let Some((k, wall_db)) = best else {
        return Bandwidth {
            cutoff_hz: None,
            wall_db: 0.0,
            nyquist_hz: nyquist,
        };
    };

    // Above a wall the spectrum stays down to Nyquist: a fall that comes
    // back is a notch or a band-limited instrument, not a codec's cut.
    // Checked past the span the floor was averaged over, where the wall
    // itself has finished falling.
    let below = mean(k.saturating_sub(gap + span), k - gap);
    let comes_back = smooth[(k + gap + span).min(top)..=top]
        .iter()
        .any(|&v| v > below - COMES_BACK_DB);
    let cutoff_hz = (wall_db >= WALL_DB && !comes_back).then_some(k as f64 * bin_hz);
    Bandwidth {
        cutoff_hz,
        wall_db,
        nyquist_hz: nyquist,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 44_100.0;

    /// Noise shaped by a gain per frequency: white noise through an FFT
    /// mask, in frames, so the test controls the spectrum exactly.
    fn shaped_noise(secs: f64, gain_db: impl Fn(f64) -> f64) -> Vec<f32> {
        let mut planner = RealFftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(FRAME);
        let inverse = planner.plan_fft_inverse(FRAME);
        let mut seed = 0x1234_5678_u32;
        let mut out = Vec::new();
        while (out.len() as f64) < secs * RATE {
            let mut block: Vec<f32> = (0..FRAME)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    (seed as f32 / u32::MAX as f32) - 0.5
                })
                .collect();
            let mut spec = forward.make_output_vec();
            forward.process(&mut block, &mut spec).unwrap();
            for (k, bin) in spec.iter_mut().enumerate() {
                let hz = k as f64 * RATE / FRAME as f64;
                *bin *= 10f32.powf((gain_db(hz) / 20.0) as f32);
            }
            let mut back = inverse.make_output_vec();
            inverse.process(&mut spec, &mut back).unwrap();
            out.extend(back.iter().map(|v| v / FRAME as f32));
        }
        out
    }

    fn measure(signal: &[f32]) -> Bandwidth {
        let mut meter = BandwidthMeter::new(RATE, 1);
        for &s in signal {
            meter.push_frame(&[s]);
        }
        meter.finish().expect("a spectrum")
    }

    /// A music-like tilt: the level falls 3 dB per octave above 1 kHz.
    fn tilt(hz: f64) -> f64 {
        -3.0 * (hz.max(1000.0) / 1000.0).log2()
    }

    #[test]
    fn a_lossy_cut_is_found_where_it_stands() {
        for cut in [16_000.0, 19_000.0, 20_000.0] {
            let b = measure(&shaped_noise(20.0, |hz| {
                tilt(hz) + if hz > cut { -60.0 } else { 0.0 }
            }));
            let found = b.cutoff_hz.expect("a wall");
            assert!((found - cut).abs() < 300.0, "{cut}: {b:?}");
        }
    }

    #[test]
    fn a_full_band_spectrum_has_no_wall() {
        let b = measure(&shaped_noise(20.0, tilt));
        assert!(b.cutoff_hz.is_none(), "{b:?}");
    }

    /// An old, dull recording: the highs fade by 12 dB per octave from
    /// 6 kHz. Low, but no wall — not a lossy file.
    #[test]
    fn a_gentle_fade_is_not_a_wall() {
        let b = measure(&shaped_noise(20.0, |hz| {
            tilt(hz) - 12.0 * (hz.max(6000.0) / 6000.0).log2()
        }));
        assert!(b.cutoff_hz.is_none(), "{b:?}");
    }

    #[test]
    fn the_port_follows_the_core() {
        use waveflow_core::analysis::bandwidth as core;
        assert_eq!(BANDWIDTH_VERSION, core::BANDWIDTH_VERSION);
        assert_eq!(WALL_DB, core::WALL_DB);
        assert_eq!(LOSSY_CUTOFF_HZ, core::LOSSY_CUTOFF_HZ);
        assert_eq!(NYQUIST_MARGIN_HZ, core::NYQUIST_MARGIN_HZ);
    }

    #[test]
    fn the_sql_threshold_is_the_constant() {
        let sql = waveflow_core::probable_fake_lossless_sql!();
        assert!(
            sql.contains(&format!(
                "MIN({}, t.sample_rate / 2.0 - {})",
                LOSSY_CUTOFF_HZ as i64, NYQUIST_MARGIN_HZ as i64
            )),
            "{sql}"
        );
    }

    #[test]
    fn silence_measures_nothing() {
        let mut meter = BandwidthMeter::new(RATE, 2);
        for _ in 0..(5.0 * RATE) as usize {
            meter.push_frame(&[0.0, 0.0]);
        }
        assert!(meter.finish().is_none());
    }

    /// Highs in opposition between the channels vanish from a mono sum,
    /// which then shows a wall at 15 kHz; channel by channel they are
    /// there, and a genuine file is not taken for a fake.
    #[test]
    fn highs_in_opposition_are_not_a_wall() {
        let lows = shaped_noise(20.0, |hz| {
            tilt(hz) + if hz > 15_000.0 { -80.0 } else { 0.0 }
        });
        let highs = shaped_noise(20.0, |hz| {
            tilt(hz) + if hz > 15_000.0 { 0.0 } else { -80.0 }
        });
        let mut meter = BandwidthMeter::new(RATE, 2);
        for (&l, &h) in lows.iter().zip(&highs) {
            meter.push_frame(&[l + h, l - h]);
        }
        let b = meter.finish().expect("a spectrum");
        assert!(b.cutoff_hz.is_none(), "{b:?}");

        let mono: Vec<f32> = lows.to_vec();
        assert!(
            measure(&mono).cutoff_hz.is_some(),
            "the sum alone has the wall"
        );
    }
}
