// @vitest-environment node
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { Song } from "../api/catalog";
import { FakeCaches } from "./fake-caches";
import {
  claimOffline,
  forgetOffline,
  isKeptOffline,
  keepOffline,
  keptSongs,
  offlineCacheName,
} from "./offline";

const song = (id: string, title: string): Song => ({
  id,
  library_id: "library",
  album_id: null,
  title,
  album: "Album",
  artist: "Artist",
  artist_id: null,
  artwork_hash: null,
  duration_ms: 1_000,
  track: 1,
  disc: 1,
  starred_at: null,
  user_rating: null,
  codec: "FLAC",
  suffix: "flac",
  size: 4,
  bitrate: null,
  sample_rate: null,
  channels: null,
  bit_depth: null,
});

describe("tracks kept for offline listening", () => {
  let store: FakeCaches;
  beforeEach(async () => {
    store = new FakeCaches();
    (globalThis as { caches?: unknown }).caches = store;
    await claimOffline("user-a");
  });
  afterEach(() => {
    delete (globalThis as { caches?: unknown }).caches;
  });

  it("stores the audio and the song by track id, never by the ticket", async () => {
    const ticket = "/api/v2/stream/wft_secret-ticket";
    const asked: string[] = [];
    const fetcher = (async (url: RequestInfo | URL) => {
      asked.push(String(url));
      return new Response(new Blob([new Uint8Array([1, 2, 3, 4])]), {
        headers: { "content-type": "audio/flac" },
      });
    }) as typeof fetch;

    await keepOffline(song("t1", "One"), ticket, fetcher);

    expect(asked).toEqual([ticket]);
    const paths = store.paths().filter((path) => path !== "/offline/owner");
    expect(paths.sort()).toEqual(["/offline/meta/t1", "/offline/tracks/t1"]);
    expect(paths.some((path) => path.includes("wft_"))).toBe(false);
    expect(paths.some((path) => path.startsWith("/api/"))).toBe(false);
    const cache = await store.open(offlineCacheName("user-a"));
    const audio = await cache.match("/offline/tracks/t1");
    expect(audio?.headers.get("content-type")).toBe("audio/flac");
    expect(new Uint8Array((await audio?.arrayBuffer()) ?? [])).toEqual(
      new Uint8Array([1, 2, 3, 4]),
    );
    const meta = await (await cache.match("/offline/meta/t1"))?.json();
    expect(meta).toMatchObject({ id: "t1", title: "One", bytes: 4 });
    // What the song is, not what the session was: nothing beyond the fields
    // a list and a player read.
    expect(Object.keys(meta).sort()).toEqual(
      [
        "album",
        "artist",
        "bytes",
        "duration_ms",
        "id",
        "kept_at",
        "replay_gain_album_gain",
        "replay_gain_album_peak",
        "replay_gain_track_gain",
        "replay_gain_track_peak",
        "suffix",
        "title",
      ].sort(),
    );
  });

  it("lists, knows and forgets what it kept", async () => {
    const fetcher = (async () =>
      new Response(new Blob([new Uint8Array([9])]))) as typeof fetch;
    await keepOffline(song("a", "First"), "/ticket-a", fetcher);
    await keepOffline(song("b", "Second"), "/ticket-b", fetcher);
    const kept = await keptSongs();
    expect(kept.map((entry) => entry.id).sort()).toEqual(["a", "b"]);
    expect(isKeptOffline("a")).toBe(true);
    await forgetOffline("a");
    expect(isKeptOffline("a")).toBe(false);
    expect((await keptSongs()).map((entry) => entry.id)).toEqual(["b"]);
    expect(store.paths().sort()).toEqual([
      "/offline/meta/b",
      "/offline/owner",
      "/offline/tracks/b",
    ]);
  });

  it("keeps one account's copies: a sign-out or another account deletes them", async () => {
    const fetcher = (async () =>
      new Response(new Blob([new Uint8Array([9])]))) as typeof fetch;
    await keepOffline(song("a", "First"), "/ticket-a", fetcher);
    // The same account renewing its session keeps them.
    await claimOffline("user-a");
    expect((await keptSongs()).map((entry) => entry.id)).toEqual(["a"]);
    // Another account signing in on this browser sees none of them.
    await claimOffline("user-b");
    expect(await keptSongs()).toEqual([]);
    expect(isKeptOffline("a")).toBe(false);
    expect(store.paths()).toEqual(["/offline/owner"]);
    await keepOffline(song("b", "Second"), "/ticket-b", fetcher);
    // A sign-out deletes them, and keeps nothing to list with no session.
    await claimOffline(null);
    expect(await keptSongs()).toEqual([]);
    expect(store.paths()).toEqual([]);
    await expect(
      keepOffline(song("c", "Third"), "/ticket-c", fetcher),
    ).rejects.toThrow();
  });

  it("stores nothing when the server refuses the audio", async () => {
    const fetcher = (async () =>
      new Response("gone", { status: 404 })) as typeof fetch;
    await expect(
      keepOffline(song("x", "Refused"), "/ticket", fetcher),
    ).rejects.toThrow();
    expect(store.paths()).toEqual(["/offline/owner"]);
  });
});
