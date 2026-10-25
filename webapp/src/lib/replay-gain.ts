/**
 * ReplayGain, as the web player applies it (RFC-022 decision 1).
 *
 * No React here: what the player needs is a number to put on a `GainNode`, and
 * the rules that produce it are the part worth testing on their own.
 */

export type ReplayGainMode = "off" | "track" | "album";

export const REPLAY_GAIN_MODES: readonly ReplayGainMode[] = [
  "off",
  "track",
  "album",
];

export type ReplayGainPreference = {
  mode: ReplayGainMode;
  /** Added to a tagged track's gain, in whole decibels. */
  preampDb: number;
};

/** The four values the scanner stores, as `SongItem` carries them. */
export type ReplayGainTags = {
  replay_gain_track_gain?: number | null;
  replay_gain_track_peak?: number | null;
  replay_gain_album_gain?: number | null;
  replay_gain_album_peak?: number | null;
};

export const PREAMP_MIN_DB = -12;
export const PREAMP_MAX_DB = 12;

/**
 * Bounds on the total, against a tag that is simply wrong. A gain of +40 dB is
 * not a loudness preference, it is a corrupt file about to deafen someone.
 */
const TOTAL_MIN_DB = -60;
const TOTAL_MAX_DB = 18;

export const DEFAULT_REPLAY_GAIN: ReplayGainPreference = {
  // Off until asked for: turning it on routes the element through an
  // AudioContext, which some mobile browsers suspend with the page.
  mode: "off",
  preampDb: 0,
};

function finite(value: number | null | undefined): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

/** A peak is a linear sample amplitude; zero or less is not a measurement. */
function usablePeak(value: number | null | undefined): number | null {
  return finite(value) && value > 0 ? value : null;
}

/**
 * The linear gain to apply to a track, 1 meaning unchanged.
 *
 * - `album` takes the album gain and peak, and falls back to the track's pair
 *   when the album gain is missing; `track` does the reverse. An album
 *   normalisation is still closer than none.
 * - A track with neither plays at unity, **without** the preamp: the preamp
 *   raises normalised tracks toward untouched ones, and adding it to an
 *   untouched track would undo exactly that.
 * - When the peak paired with the chosen gain is known, the result is capped at
 *   `1 / peak`, so a raised track never clips.
 */
export function replayGainFactor(
  tags: ReplayGainTags,
  preference: ReplayGainPreference,
): number {
  if (preference.mode === "off") return 1;
  const track = {
    gain: tags.replay_gain_track_gain,
    peak: tags.replay_gain_track_peak,
  };
  const album = {
    gain: tags.replay_gain_album_gain,
    peak: tags.replay_gain_album_peak,
  };
  const [first, second] =
    preference.mode === "album" ? [album, track] : [track, album];
  const chosen = finite(first.gain)
    ? first
    : finite(second.gain)
      ? second
      : null;
  if (!chosen || !finite(chosen.gain)) return 1;
  const preamp = finite(preference.preampDb) ? preference.preampDb : 0;
  const decibels = Math.min(
    TOTAL_MAX_DB,
    Math.max(TOTAL_MIN_DB, chosen.gain + preamp),
  );
  const linear = 10 ** (decibels / 20);
  const peak = usablePeak(chosen.peak);
  return peak === null ? linear : Math.min(linear, 1 / peak);
}

/**
 * The stored preference, or the default for anything that is not one.
 *
 * Read back from `localStorage`, which a person, an extension or an older
 * build may have written anything into. A mode it does not know is `off`, and
 * a preamp out of range is clamped rather than refused, so a stored +20 from a
 * hand edit lands on +12 instead of losing the mode beside it.
 */
export function parseReplayGainPreference(
  raw: string | null,
): ReplayGainPreference {
  if (raw === null) return DEFAULT_REPLAY_GAIN;
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return DEFAULT_REPLAY_GAIN;
  }
  if (typeof value !== "object" || value === null) return DEFAULT_REPLAY_GAIN;
  const { mode, preampDb } = value as Record<string, unknown>;
  return {
    mode: REPLAY_GAIN_MODES.includes(mode as ReplayGainMode)
      ? (mode as ReplayGainMode)
      : "off",
    preampDb: clampPreamp(typeof preampDb === "number" ? preampDb : 0),
  };
}

/** A preamp in whole decibels, inside the range the control offers. */
export function clampPreamp(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(PREAMP_MAX_DB, Math.max(PREAMP_MIN_DB, Math.round(value)));
}

/**
 * Whether a media URL may be routed through Web Audio.
 *
 * `createMediaElementSource` outputs silence for cross-origin media served
 * without CORS. Stream tickets are relative, so they are same-origin by
 * construction; this is the check that keeps a future absolute URL from
 * turning ReplayGain into a mute button.
 */
export function routableThroughWebAudio(url: string, page: string): boolean {
  try {
    return new URL(url, page).origin === new URL(page).origin;
  } catch {
    return false;
  }
}
