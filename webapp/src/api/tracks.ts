import type { Song } from "./catalog";
import { call } from "./client";

/**
 * A track with the credits the catalogue answers for it. The server sends these
 * on every `SongItem`; `Song` leaves them out because no listing reads them, and
 * only the tag editor needs them.
 */
export type SongCredits = Song & {
  year?: number | null;
  artists?: { id: string; name: string }[];
  genres?: string[];
};

export const getTrackCredits = (id: string) =>
  call<SongCredits>(`/api/v2/tracks/${id}`);

/**
 * Everything the properties panel shows for one track: what `GET
 * /api/v2/tracks/{id}` answers, which is the `SongItem` every listing carries
 * plus where the file sits in its library.
 *
 * Each field below is one the panel prints; the rest of the answer is not
 * declared, for the reason `Song` gives — a declaration is a claim the mocks
 * then confirm, so it should only be made about what is read.
 */
export type TrackProperties = SongCredits & {
  /** Relative to the library root, `/`-separated. Only this route sends it. */
  relative_path: string;
  full_hash: string;
  created_at: number;
  album_artist: string | null;
  comment: string | null;
  musicbrainz_id: string | null;
  isrc: string[];
  bpm: number | null;
  replay_gain_track_gain: number | null;
  replay_gain_track_peak: number | null;
  replay_gain_album_gain: number | null;
  replay_gain_album_peak: number | null;
};

export const getTrackProperties = (id: string) =>
  call<TrackProperties>(`/api/v2/tracks/${id}`);

/**
 * What the file's tags say, as the last scan read them. Artists and genres are
 * absent on purpose: a correction to either replaces the rows the scan wrote,
 * so the file's own credits are no longer in the database to be shown.
 */
export type TrackSourceTags = {
  title: string;
  sort_title: string | null;
  year: number | null;
  track_number: number | null;
  disc_number: number | null;
  musicbrainz_recording_id: string | null;
  comment: string | null;
};

/** The stored correction, field by field. `null` is no correction there. */
export type TrackOverrideValues = {
  title: string | null;
  sort_title: string | null;
  year: number | null;
  track_number: number | null;
  disc_number: number | null;
  musicbrainz_recording_id: string | null;
  comment: string | null;
  artists: string[] | null;
  genres: string[] | null;
};

export type TrackOverrides = {
  source: TrackSourceTags;
  overrides: TrackOverrideValues;
};

/**
 * A partial patch in three states. A key **absent** from the object leaves that
 * correction as it is, `null` removes it, and a value sets it — so a key must
 * only be present when it is meant, and never as `undefined`: `JSON.stringify`
 * would drop it, which happens to read as absent, but by accident.
 */
export type TrackCorrectionPatch = {
  [Field in keyof TrackOverrideValues]?: TrackOverrideValues[Field];
};

export const getTrackOverrides = (id: string) =>
  call<TrackOverrides>(`/api/v2/tracks/${id}/overrides`);

export const correctTrack = (id: string, patch: TrackCorrectionPatch) =>
  call<SongCredits>(`/api/v2/tracks/${id}`, {
    method: "PATCH",
    body: JSON.stringify(patch),
  });

export type LyricsLine = { start?: number; value: string };

export type StructuredLyrics = {
  displayArtist: string | null;
  displayTitle: string;
  lang: string;
  synced: boolean;
  line: LyricsLine[];
};

/**
 * What `GET /api/v2/tracks/{id}/lyrics` answers.
 *
 * `camelCase`, because `src/lyrics.rs` renames the whole struct that way — the
 * envelope as much as the `StructuredLyrics` it carries. This declared
 * `track_id`/`structured_lyrics` until 2026-09-15, so every read of it threw on
 * the first property and the playing page was unreachable. `StructuredLyrics`
 * above was already right: the inside was converted and the envelope forgotten.
 */
export type LyricsList = {
  trackId: string;
  structuredLyrics: StructuredLyrics[];
};

export const getLyrics = (trackId: string) =>
  call<LyricsList>(`/api/v2/tracks/${trackId}/lyrics`);
