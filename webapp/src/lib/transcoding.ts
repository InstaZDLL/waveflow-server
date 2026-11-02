import type { TranscodeStatus } from "../api";

/**
 * The pure half of the administration's transcoding panel.
 *
 * Kept apart from the component so the one rule in it can be held still: what
 * the server will do with the next request for a transcode.
 */

/**
 * - `unavailable`: the server cannot transcode at all, so every listener gets
 *   the original file whatever format a client asks for.
 * - `full`: every slot the server allows is taken, so a new transcode is
 *   refused with a `429` until one ends. Originals, and what is already in
 *   the transcode cache, still play.
 * - `open`: a new transcode would start.
 */
export type TranscodeLoad = "unavailable" | "full" | "open";

const FORMAT_LABELS: Record<string, string> = {
  mp3: "MP3",
  opus: "Opus",
  aac: "AAC",
};

/** How a format the wire spells `opus` is written for a reader. */
export function formatLabel(format: string): string {
  return FORMAT_LABELS[format] ?? format.toUpperCase();
}

/**
 * What a profile's select offers: what the server produces, and the format the
 * row already holds if the server no longer lists it. A controlled select with
 * no option for its value shows its first option while its state keeps the old
 * format, so saving would send what the screen does not show, and choosing the
 * option already on screen would fire no change.
 */
export function offeredFormats(formats: string[], current: string): string[] {
  return current === "raw" || formats.includes(current)
    ? formats
    : [...formats, current];
}

/**
 * The bit rate a profile's field holds: `null` when it was left empty, which
 * asks for the format's default, and `"invalid"` outside what the server takes,
 * so the page can say so in its own language before anything is sent.
 */
export function readBitrate(text: string): number | null | "invalid" {
  const trimmed = text.trim();
  if (trimmed === "") return null;
  const value = Number(trimmed);
  return Number.isInteger(value) && value >= 32 && value <= 512
    ? value
    : "invalid";
}

export function transcodeLoad(status: TranscodeStatus): TranscodeLoad {
  if (!status.available) return "unavailable";
  // At the limit, not only past it: the server refuses the request that would
  // make one more, so reaching the limit is already being full. Past it is
  // possible for a moment, because a transcode gives its slot back before its
  // count drops and another can start in between.
  return status.active >= status.global_limit ? "full" : "open";
}
