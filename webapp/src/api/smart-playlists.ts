import type { Song } from "./catalog";
import { call } from "./client";

/**
 * Smart playlists (RFC-025): a playlist defined by rules, which the server
 * evaluates every time it is read. There is no track list to edit; there are
 * rules, and what they answer now.
 *
 * The vocabulary below is the server's allowlist, written out. A value outside
 * it is refused with 422, so the editor only ever offers what is here.
 */
export type SmartField =
  | "title"
  | "album"
  | "artist"
  | "album_artist"
  | "genre"
  | "year"
  | "rating"
  | "starred"
  | "play_count"
  | "last_played"
  | "date_added"
  | "duration"
  | "bitrate"
  | "format"
  | "library";

export type SmartOperator =
  | "is"
  | "is_not"
  | "contains"
  | "not_contains"
  | "starts_with"
  | "ends_with"
  | "greater_than"
  | "less_than"
  | "between"
  | "in_the_last"
  | "not_in_the_last"
  | "before"
  | "after";

export type SmartSort =
  | "title"
  | "artist"
  | "album"
  | "year"
  | "rating"
  | "play_count"
  | "last_played"
  | "date_added"
  | "duration"
  | "bitrate"
  | "random";

export type SmartConditionValue = string | number | boolean | [number, number];

export type SmartCondition = {
  field: SmartField;
  operator: SmartOperator;
  value: SmartConditionValue;
};

export type SmartRules = {
  match: "all" | "any";
  conditions: SmartCondition[];
  sort: SmartSort | null;
  order: "asc" | "desc";
  /** At most 1000; that many when `null`. */
  limit: number | null;
};

export type SmartPlaylist = {
  id: string;
  owner_id: string;
  owner_username: string;
  name: string;
  comment: string | null;
  public: boolean;
  rules: SmartRules;
  created_at: number;
  updated_at: number;
  /** When the answer was computed: a smart playlist stores no track list. */
  evaluated_at: number;
  /** What this account can play of it. */
  song_count: number;
  duration_ms: number;
  /** On a read of one playlist; absent from a listing. */
  songs?: Song[];
};

export type SmartPreview = {
  song_count: number;
  duration_ms: number;
  songs: Song[];
};

export type SmartPlaylistDraft = {
  name: string;
  comment?: string | null;
  public?: boolean;
  rules: SmartRules;
};

/** This account's smart playlists, then other accounts' public ones. */
export const listSmartPlaylists = () =>
  call<SmartPlaylist[]>("/api/v2/smart-playlists?include_public=true");

export const getSmartPlaylist = (id: string) =>
  call<SmartPlaylist>(`/api/v2/smart-playlists/${id}`);

export const createSmartPlaylist = (draft: SmartPlaylistDraft) =>
  call<SmartPlaylist>("/api/v2/smart-playlists", {
    method: "POST",
    body: JSON.stringify(draft),
  });

/** `rules`, when sent, replaces the whole rule set. */
export const updateSmartPlaylist = (
  id: string,
  patch: Partial<SmartPlaylistDraft>,
) =>
  call<SmartPlaylist>(`/api/v2/smart-playlists/${id}`, {
    method: "PATCH",
    body: JSON.stringify(patch),
  });

export const deleteSmartPlaylist = (id: string) =>
  call<void>(`/api/v2/smart-playlists/${id}`, { method: "DELETE" });

/** What rules would answer for this account, without storing anything. */
export const previewSmartPlaylist = (rules: SmartRules) =>
  call<SmartPreview>("/api/v2/smart-playlists/preview", {
    method: "POST",
    body: JSON.stringify({ rules }),
  });
