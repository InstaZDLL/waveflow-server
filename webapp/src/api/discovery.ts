/**
 * What the listening history and the catalogue suggest (RFC-024): the
 * statistics page, the mixes on the home page, and similar songs.
 */

import type { Album, Song } from "./catalog";
import { call, scoped } from "./client";

/** The periods `GET /api/v2/stats` accepts, spelled as the server does. */
export type StatsPeriod = "7d" | "30d" | "365d" | "all";

export type StatsBucket = {
  /** `YYYY-MM-DD` or `YYYY-MM`, in the listener's time. */
  label: string;
  plays: number;
  listening_ms: number;
};

export type ListeningStats = {
  period: StatsPeriod;
  /** Where the period starts, in epoch milliseconds; `null` for `all`. */
  from: number | null;
  bucket: "day" | "month";
  totals: {
    plays: number;
    /** An estimate: the listened tracks' durations, summed. */
    listening_ms: number;
    tracks: number;
    albums: number;
    artists: number;
  };
  top_tracks: Array<{ song: Song; plays: number }>;
  top_albums: Array<{ album: Album; plays: number }>;
  top_artists: Array<{
    artist: { id: string; name: string };
    artwork_hash: string | null;
    plays: number;
  }>;
  top_genres: Array<{ name: string; plays: number }>;
  timeline: StatsBucket[];
  first_play: { song: Song; played_at: number } | null;
  last_play: { song: Song; played_at: number } | null;
};

/** What a client names a mix by; the server sends no display text. */
export type MixKind =
  | "most_played"
  | "never_played"
  | "forgotten_favourites"
  | "recently_added"
  | "genre"
  | "decade";

export type MixSummary = {
  /** What `GET /api/v2/mixes/{id}` takes. */
  id: string;
  kind: MixKind;
  /** The genre's display name, for a genre mix. */
  genre: string | null;
  /** The decade's first year, for a decade mix. */
  decade: number | null;
  track_count: number;
  /** Up to four distinct covers from the mix, in its order. */
  artwork_hashes: string[];
};

export type Mix = MixSummary & { songs: Song[] };

/**
 * How far this browser is from UTC, in minutes, as the server reads it: east
 * is positive, which is the opposite of `getTimezoneOffset`.
 */
export function utcOffsetMinutes(at: Date = new Date()): number {
  return -at.getTimezoneOffset();
}

/** One account's listening over `period`, cut into the listener's days. */
export const getListeningStats = (period: StatsPeriod) =>
  call<ListeningStats>(
    `/api/v2/stats?${new URLSearchParams({
      period,
      utc_offset_minutes: String(utcOffsetMinutes()),
    })}`,
  );

/** The mixes this account has today, in the active library. */
export const listMixes = (libraryId?: string) =>
  call<MixSummary[]>(
    `/api/v2/mixes?${new URLSearchParams(
      scoped(libraryId, { utc_offset_minutes: String(utcOffsetMinutes()) }),
    )}`,
  );

/**
 * One mix and its tracks. The id is a path segment that may hold a genre's
 * name, so it is encoded whole.
 */
export const getMix = (id: string, libraryId?: string) =>
  call<Mix>(
    `/api/v2/mixes/${encodeURIComponent(id)}?${new URLSearchParams(
      scoped(libraryId, { utc_offset_minutes: String(utcOffsetMinutes()) }),
    )}`,
  );

/** Songs like the artist, album or track behind `id`. */
export const listSimilarSongs = (id: string, limit = 50) =>
  call<Song[]>(
    `/api/v2/similar-songs/${id}?${new URLSearchParams({ limit: String(limit) })}`,
  );
