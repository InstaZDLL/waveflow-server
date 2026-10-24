import type { QualityVerdict } from "../api";

/**
 * The pure half of the quality page and of a track's "Real quality" section
 * (RFC-016): the reading of a verdict, and the averaged spectrum drawn as a
 * line. Kept apart from the components so a test can hold the drawing still.
 */

/**
 * What a verdict accuses a file of. A finding is drawn as a warning; the
 * rest are states the analysis is in, or a verdict about nothing.
 */
export function verdictTone(
  verdict: QualityVerdict,
): "finding" | "mild" | "neutral" {
  switch (verdict) {
    case "lossy_source":
    case "upsampled":
      return "finding";
    case "padded":
    case "unreadable":
      return "mild";
    default:
      return "neutral";
  }
}

/**
 * The frequencies a reader compares a wall against, as RFC-016 decision 8
 * names them: where LAME cuts at 128, 192 and 320 kbps, and where a CD's own
 * filter stands. Only those under the file's Nyquist.
 */
export function spectrumGridlines(nyquistHz: number): number[] {
  return [16_000, 19_000, 20_000, 22_000].filter((hz) => hz < nyquistHz);
}

/** The level the chart reaches down to, in dB under the loudest band. */
export const SPECTRUM_FLOOR_DB = -120;

/**
 * The averaged spectrum as an SVG path in a `width` × `height` box: 0 Hz at
 * the left, Nyquist at the right, the loudest band at the top and
 * {@link SPECTRUM_FLOOR_DB} at the bottom. Empty for an empty spectrum.
 */
export function spectrumPath(
  spectrum: number[],
  width: number,
  height: number,
): string {
  if (spectrum.length === 0) return "";
  const step = spectrum.length > 1 ? width / (spectrum.length - 1) : 0;
  return spectrum
    .map((db, index) => {
      const level = Math.min(0, Math.max(SPECTRUM_FLOOR_DB, db));
      const x = index * step;
      const y = (level / SPECTRUM_FLOOR_DB) * height;
      return `${index === 0 ? "M" : "L"}${round(x)} ${round(y)}`;
    })
    .join(" ");
}

/** Where a frequency stands across the chart, from 0 to `width`. */
export function frequencyX(hz: number, nyquistHz: number, width: number) {
  if (nyquistHz <= 0) return 0;
  return round(Math.min(1, Math.max(0, hz / nyquistHz)) * width);
}

function round(value: number): number {
  return Math.round(value * 10) / 10;
}
