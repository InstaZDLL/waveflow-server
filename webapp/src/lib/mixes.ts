/**
 * How a mix and a listening period are named, and how listening time reads.
 *
 * The server sends a mix's kind and its parameters, never a display text: the
 * words are the client's, in the listener's language (RFC-024 decision 7).
 */

import type { MixSummary } from "../api";
import type { TranslationKey } from "../state/i18n";

type Translate = (
  key: TranslationKey,
  values?: Record<string, string | number>,
) => string;

const KIND_LABELS: Record<
  Exclude<MixSummary["kind"], "genre" | "decade">,
  TranslationKey
> = {
  most_played: "mixes.mostPlayed",
  never_played: "mixes.neverPlayed",
  forgotten_favourites: "mixes.forgottenFavourites",
  recently_added: "mixes.recentlyAdded",
};

/** The title a mix is shown under. */
export function mixTitle(mix: MixSummary, t: Translate): string {
  switch (mix.kind) {
    case "genre":
      return t("mixes.genre", { genre: mix.genre ?? "" });
    case "decade":
      return t("mixes.decade", { decade: mix.decade ?? 0 });
    default:
      return t(KIND_LABELS[mix.kind]);
  }
}

/**
 * Listening time as a person says it: hours and minutes, or minutes alone
 * under an hour. Rounded to the minute, since the server's figure is an
 * estimate to begin with.
 */
export function listeningTime(ms: number, t: Translate): string {
  const minutes = Math.round(ms / 60_000);
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  if (hours === 0) return t("stats.minutes", { count: rest });
  return t("stats.hoursMinutes", { hours, minutes: rest });
}

/**
 * Each bucket's share of the busiest one, from 0 to 1, for the bars. An empty
 * timeline is all zeroes rather than a division by nothing.
 */
export function barShares(plays: number[]): number[] {
  const most = Math.max(0, ...plays);
  return plays.map((count) => (most === 0 ? 0 : count / most));
}
