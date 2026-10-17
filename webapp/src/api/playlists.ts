import type { Song } from "./catalog";
import { call, request } from "./client";

export type Playlist = {
  id: string;
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
  /** How the server read the bytes: `.m3u` is frequently not UTF-8. */
  encoding: "utf-8" | "windows-1252";
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
  reason: "no-match" | "ambiguous" | "not-a-path" | "duplicate";
};

export const listPlaylists = () => call<Playlist[]>("/api/v2/playlists");

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
