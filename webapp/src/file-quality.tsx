import type { Song } from "./api";
import { type TranslationKey, useI18n } from "./i18n";

/**
 * What a file is, and a meter that says it at a glance.
 *
 * Apart from `pages.tsx` for the reason `page-shell.tsx` is: this needs
 * `useI18n` and a `Song`, and nothing else. A screen that wants it should not
 * acquire a music player along the way.
 */

/** The bands the meter lights, weakest first. */
export type QualityBand =
  | "unknown"
  | "low"
  | "standard"
  | "high"
  | "lossless"
  | "hiRes";

const BANDS: QualityBand[] = ["low", "standard", "high", "lossless", "hiRes"];

/** How many of the five steps a band lights. */
export function qualitySteps(band: QualityBand): number {
  const index = BANDS.indexOf(band);
  return index < 0 ? 0 : index + 1;
}

export const QUALITY_STEPS = BANDS.length;

/**
 * Which band a file falls in.
 *
 * **`bit_depth` is what says "lossless", and the server is what says
 * `bit_depth`.** The scanner reads it from the file's properties and keeps it
 * only when it is above zero (`src/scanner.rs`), and a format without one —
 * every lossy codec — reports nothing. So the client does not keep a list of
 * codec names to decide this: such a list is a vocabulary the reader on the
 * server already has, and a copy here would be out of date the first time
 * somebody scans a format nobody thought of.
 *
 * Read with `typeof` rather than against `null`, and that is not pedantry:
 * the declaration in `api.ts` says `number | null`, and a declaration is a
 * claim about the server rather than a guarantee from it. A response missing
 * the key entirely gives `undefined`, which `!== null` waves through — this
 * function would then have called a file lossless on the strength of a field
 * it never received. `typeof x === "number"` is true of a measurement and of
 * nothing else.
 *
 * Below that, the grading is the bitrate, which is the only axis a lossy file
 * offers. The thresholds are conventional rather than measured — 256 and 128
 * kbps — and they are a summary, not a verdict: Opus at 128 is transparent
 * where MP3 at 128 is audibly not, and no field here can tell them apart. The
 * numbers beside the meter are the honest part; the meter is the glance.
 */
export function qualityBand(song: Song): QualityBand {
  if (typeof song.bit_depth === "number") {
    const deep = song.bit_depth > 16;
    const fast =
      typeof song.sample_rate === "number" && song.sample_rate > 48_000;
    return deep || fast ? "hiRes" : "lossless";
  }
  if (typeof song.bitrate !== "number") return "unknown";
  if (song.bitrate >= 256) return "high";
  if (song.bitrate >= 128) return "standard";
  return "low";
}

const BAND_LABEL: Record<QualityBand, TranslationKey> = {
  unknown: "quality.unknown",
  low: "quality.low",
  standard: "quality.standard",
  high: "quality.high",
  lossless: "quality.lossless",
  hiRes: "quality.hiRes",
};

/**
 * What to call the container, in upper case, or `null` if there is nothing.
 *
 * `||` and not `??`: a codec that arrived as an empty string is not a codec,
 * and `??` keeps it — the line then lost its container name altogether
 * instead of falling back to the suffix, which the server never leaves
 * absent. Non-strings are read as missing for the reason the numbers are:
 * this type is a claim about the server, not a guarantee from it.
 */
export function containerName(song: Song): string | null {
  const text = [song.codec, song.suffix].find(
    (value) => typeof value === "string" && value.trim() !== "",
  );
  return text ? text.trim().toUpperCase() : null;
}

/** A sample rate in kHz, trimmed: 44.1, and 48 rather than 48.0. */
function kilohertz(rate: number): string {
  const value = rate / 1000;
  return Number.isInteger(value) ? String(value) : value.toFixed(1);
}

/** Bytes as the unit a person reads: MB above a megabyte, kB below. */
function fileSize(bytes: number): string {
  return bytes >= 1024 * 1024
    ? `${(bytes / (1024 * 1024)).toFixed(1)} MB`
    : `${Math.round(bytes / 1024)} kB`;
}

/**
 * The facts, in one line, and the meter above them.
 *
 * No per-field labels: a unit says what a number is, and six labelled rows
 * would be a table where a listener wants a glance. The container is named
 * from `codec` when the reader supplied one and from `suffix` otherwise —
 * `suffix` is never null, so the line always starts with something true.
 */
export function FileQuality({ song }: { song: Song }) {
  const { t } = useI18n();
  const band = qualityBand(song);
  const steps = qualitySteps(band);
  // Same `typeof` reading as the band, and for the same reason: a key the
  // response did not carry must leave its fact out, not print `undefined`
  // kbps.
  const measured = (value: number | null) =>
    typeof value === "number" ? value : null;
  const bitrate = measured(song.bitrate);
  const rate = measured(song.sample_rate);
  const depth = measured(song.bit_depth);
  const channels = measured(song.channels);
  const size = measured(song.size);
  const facts = [
    containerName(song),
    bitrate === null ? null : `${bitrate} kbps`,
    rate === null ? null : `${kilohertz(rate)} kHz`,
    depth === null ? null : `${depth} bit`,
    channels === null ? null : t("file.channels", { count: channels }),
    size === null ? null : fileSize(size),
  ].filter((fact): fact is string => fact !== null);

  return (
    <div className="file-quality">
      {/* Decorative, and `aria-hidden` for it. The band is printed in
          words immediately below, so a reader hears "Lossless · FLAC · 957
          kbps" rather than a meter announcing four of five — which is the
          same fact, said worse. A `role="meter"` here would have been the
          third telling. */}
      <div className="quality-meter" aria-hidden="true">
        {BANDS.map((step, index) => (
          <span key={step} className={index < steps ? "on" : undefined} />
        ))}
      </div>
      <p className="muted file-facts">
        <strong>{t(BAND_LABEL[band])}</strong>
        <span> · {facts.join(" · ")}</span>
      </p>
    </div>
  );
}
