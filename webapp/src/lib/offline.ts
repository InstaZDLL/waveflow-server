/**
 * Tracks kept for offline listening (RFC-022, offline playback).
 *
 * A kept track is two entries in the cache of **the account that kept it**,
 * both under addresses of the client's own that name the track and carry no
 * credential:
 *
 * - `/offline/tracks/{id}`, the audio, which the service worker (`/sw.js`)
 *   answers with byte ranges so the player can seek;
 * - `/offline/meta/{id}`, the song as the catalogue described it when it was
 *   kept, so the offline page can list it with no server to ask.
 *
 * **The copies are one account's.** Each account keeps them in a cache named
 * after its id, and the browser remembers which account's are current in an
 * owner marker that the page and the worker both read. A sign-out deletes
 * them; a different account signing in deletes the previous one's before it
 * is shown anything; with no marker, nothing is listed or played. What a
 * shared or family computer keeps is therefore only ever the copies of the
 * account that last signed in on it and has not signed out — the same reach
 * as that account's session left open.
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

/** The prefix of every account's cache of kept tracks. */
export const OFFLINE_PREFIX = "waveflow-offline-v1:";
/** Where the browser remembers whose kept tracks are current. */
export const OFFLINE_OWNER_CACHE = "waveflow-offline-owner";
const OWNER_KEY = "/offline/owner";

export const offlineCacheName = (owner: string) => `${OFFLINE_PREFIX}${owner}`;

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

/** The account whose kept tracks are current, or `null` for none. */
export async function offlineOwner(): Promise<string | null> {
  if (!offlineSupported()) return null;
  const marker = await (await caches.open(OFFLINE_OWNER_CACHE)).match(
    OWNER_KEY,
  );
  const owner = (await marker?.text())?.trim();
  return owner ? owner : null;
}

/** Deletes every account's kept tracks, and the marker. */
async function deleteAllCopies(): Promise<void> {
  for (const name of await caches.keys()) {
    if (name.startsWith(OFFLINE_PREFIX)) await caches.delete(name);
  }
  await caches.delete(OFFLINE_OWNER_CACHE);
}

/**
 * Says whose session this browser now holds: `userId` on a sign-in or a
 * renewal, `null` on a sign-out.
 *
 * A sign-out deletes every kept copy — the safer default on a computer
 * someone else may use next, and what the offline page tells the listener.
 * An account that is not the one whose copies are current deletes them before
 * it can see them, then becomes the owner.
 */
export async function claimOffline(userId: string | null): Promise<void> {
  if (!offlineSupported()) return;
  const current = await offlineOwner();
  // A renewal of the same account's session changes nothing: what it kept
  // stays known, mid-session, as it was.
  if (userId !== null && userId === current) return;
  if (userId === null || current !== null) await deleteAllCopies();
  if (userId !== null) {
    await (await caches.open(OFFLINE_OWNER_CACHE)).put(
      OWNER_KEY,
      new Response(userId, { headers: { "content-type": "text/plain" } }),
    );
  }
  known = null;
  changed();
}

/** Forgets what was read of the kept tracks, so it is read again. */
export function forgetKnownOffline(): void {
  known = null;
  changed();
}

async function ownerCache(): Promise<Cache | null> {
  const owner = await offlineOwner();
  return owner ? caches.open(offlineCacheName(owner)) : null;
}

/**
 * Keeps one track for the account whose session this is: its audio, fetched
 * from `audioUrl` — a stream ticket's address, used once and not stored — and
 * what its song says.
 *
 * Asks the browser to keep the storage rather than evict it under pressure:
 * a listener who kept an album for a flight means it.
 */
export async function keepOffline(
  song: Song,
  audioUrl: string,
  fetcher: typeof fetch = fetch,
): Promise<void> {
  const cache = await ownerCache();
  if (!cache) throw new Error("no account holds this browser's offline copies");
  const response = await fetcher(audioUrl, {
    credentials: "same-origin",
    cache: "no-store",
  });
  if (!response.ok) throw new Error(`offline copy refused: ${response.status}`);
  const audio = await response.blob();
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
  const cache = await ownerCache();
  if (cache) {
    await cache.delete(offlineTrackUrl(trackId));
    await cache.delete(metaUrl(trackId));
  }
  known?.delete(trackId);
  changed();
}

/** Every track the current account kept, the most recently kept first. */
export async function keptSongs(): Promise<KeptSong[]> {
  if (!offlineSupported()) return [];
  const cache = await ownerCache();
  if (!cache) {
    known = new Set();
    return [];
  }
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
 * byte ranges from the current account's cache. Without one — the first
 * load, a private window — an object URL over the stored file, which the
 * caller revokes; seeking works on it too, from memory.
 */
export async function offlineSource(
  trackId: string,
): Promise<{ url: string; revoke: () => void } | null> {
  const cache = await ownerCache();
  const response = await cache?.match(offlineTrackUrl(trackId));
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
