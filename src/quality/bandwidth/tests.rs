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
