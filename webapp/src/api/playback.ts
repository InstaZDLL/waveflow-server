import type { Song } from "./catalog";
import { call } from "./client";

export type Queue = {
  current: string | null;
  position_ms: number;
  changed_by: string | null;
  updated_at: number;
  songs: Song[];
};

export const getQueue = () => call<Queue | null>("/api/v2/queue");

export const saveQueue = (
  songs: Song[],
  current: string | null,
  positionMs: number,
) =>
  call<void>("/api/v2/queue", {
    method: "PUT",
    body: JSON.stringify({
      track_ids: songs.map((song) => song.id),
      current,
      position_ms: positionMs,
      client: "WaveFlow Web",
    }),
  });

export type NowPlaying = {
  username: string;
  song: Song;
  started_at: number;
};

/** List the playback sessions currently reported by the server. */
export const listNowPlaying = () => call<NowPlaying[]>("/api/v2/now-playing");

/**
 * Whether this server can transcode, and how much of its room for it is taken.
 *
 * `formats` lists what `format=` may ask for besides `raw`, which is always
 * accepted and is not a transcode. Past `global_limit` transcodes running at
 * once, or `per_user_limit` for one account, the stream routes refuse a new
 * one with a `429`.
 */
export type TranscodeStatus = {
  available: boolean;
  active: number;
  global_limit: number;
  per_user_limit: number;
  formats: string[];
};

export const getTranscodeStatus = () =>
  call<TranscodeStatus>("/api/v2/transcode/status");

export type Bookmark = {
  position_ms: number;
  comment: string | null;
  created_at: number;
  updated_at: number;
  song: Song;
};

export const listBookmarks = () => call<Bookmark[]>("/api/v2/bookmarks");

/**
 * One bookmark per account and track, so this replaces rather than adds — the
 * route is `PUT` for that reason, and sending the same position twice leaves
 * the same single bookmark.
 */
export const setBookmark = (
  trackId: string,
  positionMs: number,
  comment?: string,
) =>
  call<void>(`/api/v2/bookmarks/${trackId}`, {
    method: "PUT",
    body: JSON.stringify({
      position_ms: Math.max(0, Math.round(positionMs)),
      comment: comment ?? null,
    }),
  });

export const deleteBookmark = (trackId: string) =>
  call<void>(`/api/v2/bookmarks/${trackId}`, { method: "DELETE" });

/**
 * `GET /history` answers plays, not songs — `track_id`, `submission` and
 * `played_at` — so a screen wanting titles resolves them itself. The default
 * limit is the server's 200; the cap is `MAX_SYNC_LIMIT`.
 */
export type Play = {
  track_id: string;
  submission: boolean;
  played_at: number;
};

export const listHistory = (limit = 100) =>
  call<Play[]>(`/api/v2/history?limit=${limit}`);
