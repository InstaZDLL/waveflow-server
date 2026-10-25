/**
 * Tracks kept for offline listening (RFC-022, offline playback).
 *
 * A kept track is two entries in one Cache Storage cache, both under addresses
 * of the client's own that name the track and carry no credential:
 *
 * - `/offline/tracks/{id}`, the audio, which the service worker (`/sw.js`)
 *   answers with byte ranges so the player can seek;
 * - `/offline/meta/{id}`, the song as the catalogue described it when it was
 *   kept, so the offline page can list it with no server to ask.
 *
 * **The audio is fetched through a stream ticket and stored under the id**,
 * never under the ticket's address: a ticket is a credential, and an hour from
 * now it opens nothing anyway. No API response is stored here either — the
 * song is written field by field from what the page already holds.
 *
 * No React here: the page and the menu call these, and the tests drive them
 * against a cache of their own.
 */

import type { Song } from "../api/catalog";

export const OFFLINE_CACHE = "waveflow-offline-v1";

export const offlineTrackUrl = (trackId: string) =>
  `/offline/tracks/${encodeURIComponent(trackId)}`;
const metaUrl = (trackId: string) =>
  `/offline/meta/${encodeURIComponent(trackId)}`;

/** Whether this browser can keep anything at all. */
export function offlineSupported(): boolean {
  return typeof caches !== "undefined";
}

/** What a kept track remembers of its song: what a list and a player read. */
export type KeptSong = Pick<
  Song,
  | "id"
  | "title"
  | "artist"
  | "album"
  | "duration_ms"
  | "suffix"
  | "replay_gain_track_gain"
  | "replay_gain_track_peak"
  | "replay_gain_album_gain"
  | "replay_gain_album_peak"
> & { kept_at: number; bytes: number };

function remembered(song: Song, bytes: number): KeptSong {
  return {
    id: song.id,
    title: song.title,
    artist: song.artist,
    album: song.album,
    duration_ms: song.duration_ms,
    suffix: song.suffix,
    replay_gain_track_gain: song.replay_gain_track_gain ?? null,
    replay_gain_track_peak: song.replay_gain_track_peak ?? null,
    replay_gain_album_gain: song.replay_gain_album_gain ?? null,
    replay_gain_album_peak: song.replay_gain_album_peak ?? null,
    kept_at: Date.now(),
    bytes,
  };
}

const listeners = new Set<() => void>();
let known: Set<string> | null = null;

/** Called whenever what is kept changes. Answers the unsubscription. */
export function onOfflineChange(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function changed() {
  for (const listener of listeners) listener();
}

/**
 * Keeps one track: its audio, fetched from `audioUrl` — a stream ticket's
 * address, used once and not stored — and what its song says.
 *
 * Asks the browser to keep the storage rather than evict it under pressure:
 * a listener who kept an album for a flight means it.
 */
export async function keepOffline(
  song: Song,
  audioUrl: string,
  fetcher: typeof fetch = fetch,
): Promise<void> {
  const response = await fetcher(audioUrl, {
    credentials: "same-origin",
    cache: "no-store",
  });
  if (!response.ok) throw new Error(`offline copy refused: ${response.status}`);
  const audio = await response.blob();
  const cache = await caches.open(OFFLINE_CACHE);
  await cache.put(
    offlineTrackUrl(song.id),
    new Response(audio, {
      headers: {
        "content-type": audio.type || "application/octet-stream",
        "content-length": String(audio.size),
      },
    }),
  );
  await cache.put(
    metaUrl(song.id),
    new Response(JSON.stringify(remembered(song, audio.size)), {
      headers: { "content-type": "application/json" },
    }),
  );
  known?.add(song.id);
  changed();
  try {
    await navigator.storage?.persist?.();
  } catch {
    // A browser that will not promise is still a browser that stored it.
  }
}

/** Lets one kept track go. */
export async function forgetOffline(trackId: string): Promise<void> {
  const cache = await caches.open(OFFLINE_CACHE);
  await cache.delete(offlineTrackUrl(trackId));
  await cache.delete(metaUrl(trackId));
  known?.delete(trackId);
  changed();
}

/** Every kept track, the most recently kept first. */
export async function keptSongs(): Promise<KeptSong[]> {
  if (!offlineSupported()) return [];
  const cache = await caches.open(OFFLINE_CACHE);
  const songs: KeptSong[] = [];
  for (const request of await cache.keys()) {
    const path = new URL(request.url, "http://offline.invalid").pathname;
    if (!path.startsWith("/offline/meta/")) continue;
    const response = await cache.match(request);
    try {
      songs.push((await response?.json()) as KeptSong);
    } catch {
      // A copy written by a build that is no longer here, or by hand.
    }
  }
  known = new Set(songs.map((song) => song.id));
  return songs.sort((left, right) => right.kept_at - left.kept_at);
}

/**
 * Whether a track is kept, from what was last read: synchronous, for the
 * menu and the player, and filled by the first [`keptSongs`] call.
 */
export function isKeptOffline(trackId: string): boolean {
  return known?.has(trackId) ?? false;
}

/**
 * The address a kept track plays from.
 *
 * Under a service worker, the worker's own address, which it answers with
 * byte ranges from the cache. Without one — the first load, a private window
 * — an object URL over the stored file, which the caller revokes; seeking
 * works on it too, from memory.
 */
export async function offlineSource(
  trackId: string,
): Promise<{ url: string; revoke: () => void } | null> {
  const cache = await caches.open(OFFLINE_CACHE);
  const response = await cache.match(offlineTrackUrl(trackId));
  if (!response) return null;
  if (navigator.serviceWorker?.controller) {
    return { url: offlineTrackUrl(trackId), revoke: () => undefined };
  }
  const url = URL.createObjectURL(await response.blob());
  return { url, revoke: () => URL.revokeObjectURL(url) };
}

/**
 * Registers the service worker that keeps the shell and answers the kept
 * tracks. A production build only: in development Vite serves the client,
 * and a worker holding its modules would serve yesterday's code.
 */
export function registerServiceWorker(production: boolean): void {
  if (!production || !("serviceWorker" in navigator)) return;
  window.addEventListener("load", () => {
    navigator.serviceWorker
      .register("/sw.js", { scope: "/" })
      .catch(() => undefined);
  });
}
