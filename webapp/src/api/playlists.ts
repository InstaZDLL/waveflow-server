import type { Song } from "./catalog";
import { call, request } from "./client";

export type Playlist = {
  id: string;
  /**
   * Whose playlist it is. Another account's public playlist is listed beside
   * this account's own (RFC-020), and only its owner may change it: the
   * server answers anyone else 404.
   */
  owner_id: string;
  owner_username: string;
  name: string;
  comment: string | null;
  public: boolean;
  created_at: number;
  updated_at: number;
  songs: Song[];
};

/**
 * What an import made of a playlist file.
 *
 * `unresolved` is the point of it. An import that quietly produced a shorter
 * playlist would be worse than a refused one, so the server names every entry
 * it could not place, and a dry run asks for the whole answer before anything
 * is created.
 */
export type PlaylistImport = {
  /** The playlist as created; `null` for a dry run. */
  playlist: Playlist | null;
  name: string;
  /**
   * How the server read the bytes: `.m3u` is frequently not UTF-8. The
   * encoding's WHATWG name in lower case — `utf-8`, `utf-16le`,
   * `windows-1252`, `shift_jis`, `gbk`… — so an open set, not a union.
   */
  encoding: string;
  /** Targets the file named, placed or not. */
  entries: number;
  resolved: number;
  unresolved: UnresolvedEntry[];
};

export type UnresolvedEntry = {
  /** One-based line in the file, so a report reads against it. */
  line: number;
  entry: string;
  title: string | null;
  /**
   * `duplicate` is no longer produced: a file naming a track twice is
   * imported with both copies. It stays here because the schema keeps it.
   */
  reason: "no-match" | "ambiguous" | "not-a-path" | "duplicate";
};

export const listPlaylists = () => call<Playlist[]>("/api/v2/playlists");

/**
 * This account's playlists, then every other account's public ones, each with
 * its tracks read through this account's libraries.
 */
export const listVisiblePlaylists = () =>
  call<Playlist[]>("/api/v2/playlists?include_public=true");

/** Publishes a playlist to every account on the server, or withdraws it. */
export const setPlaylistPublic = (id: string, isPublic: boolean) =>
  call<Playlist>(`/api/v2/playlists/${id}`, {
    method: "PATCH",
    body: JSON.stringify({ public: isPublic }),
  });

export const createPlaylist = (name: string, trackIds: string[] = []) =>
  call<Playlist>("/api/v2/playlists", {
    method: "POST",
    body: JSON.stringify({ name, track_ids: trackIds }),
  });

/**
 * Sends a playlist file for import, or for a dry run.
 *
 * The file goes up as the bytes it is: its encoding is part of the problem —
 * `.m3u` is frequently not UTF-8 — and decoding it here is the one decision
 * this client cannot make correctly. Hence the explicit content type, which is
 * also why `request` only defaults to JSON when a caller says nothing.
 *
 * `dryRun` resolves and reports without creating anything, which is what the
 * import panel shows before it commits: an import that silently lost a third of
 * a collection would be worse than one that refused.
 */
export const importPlaylist = (
  file: File,
  options: { name?: string; libraryId?: string; dryRun?: boolean } = {},
) => {
  const query = new URLSearchParams();
  if (options.name) query.set("name", options.name);
  if (options.libraryId) query.set("library_id", options.libraryId);
  if (options.dryRun) query.set("dry_run", "true");
  return call<PlaylistImport>(`/api/v2/playlists/import?${query}`, {
    method: "POST",
    headers: { "content-type": "audio/x-mpegurl" },
    body: file,
  });
};

/** One playlist as an extended `.m3u8`, for the caller to save. */
export const exportPlaylist = async (id: string) =>
  (await request(`/api/v2/playlists/${id}/export`)).text();

export const deletePlaylist = (id: string) =>
  call<void>(`/api/v2/playlists/${id}`, { method: "DELETE" });

export const appendToPlaylist = (id: string, trackIds: string[]) =>
  call<Playlist>(`/api/v2/playlists/${id}`, {
    method: "PATCH",
    body: JSON.stringify({ add: trackIds }),
  });

/**
 * Moves the entry shown at `from` so it is shown at `to`. `trackId` is the
 * track the caller believes sits at `from`: the server refuses the move (409)
 * when it does not, rather than moving another entry of a list that changed.
 */
export const movePlaylistEntry = (
  id: string,
  from: number,
  to: number,
  trackId: string,
) =>
  call<Playlist>(`/api/v2/playlists/${id}/move`, {
    method: "POST",
    body: JSON.stringify({ from, to, track_id: trackId }),
  });

/**
 * Removes one entry, by the position it is shown at: a playlist may hold a
 * track twice, and the position is what says which copy goes. `trackId` is
 * the track the caller believes is there, guarded as a move is (409).
 */
export const removePlaylistEntry = (
  id: string,
  position: number,
  trackId: string,
) =>
  call<Playlist>(`/api/v2/playlists/${id}/remove`, {
    method: "POST",
    body: JSON.stringify({ position, track_id: trackId }),
  });
