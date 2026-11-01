import { describe, expect, it } from "vitest";

import {
  albumQuery,
  albumsQuery,
  artistQuery,
  artistsQuery,
  attentionSummaryQuery,
  favoriteTracksQuery,
  lyricsQuery,
  playlistsQuery,
  randomSongsQuery,
  smartPlaylistQuery,
  smartPlaylistsQuery,
  songsQuery,
  trackEditQuery,
  trackPropertiesQuery,
} from "../api/queries";
import { everythingIn, staleAfter } from "./library-events";

/**
 * Read through the query options themselves rather than spelt out here: a key
 * renamed in `api/queries/` must break these tests, not leave them asserting a
 * spelling nobody uses.
 */
const key = (options: { queryKey: readonly unknown[] }) => options.queryKey;

const track = (id: string) => ({
  entity_type: "track" as const,
  entity_id: id,
});
const album = (id: string) => ({
  entity_type: "album" as const,
  entity_id: id,
});
const artist = (id: string) => ({
  entity_type: "artist" as const,
  entity_id: id,
});

describe("staleAfter", () => {
  it("marks the library's listings and the unscoped ones, not another library's", () => {
    const stale = staleAfter("lib-a", [track("t1")]);
    expect(stale(key(albumsQuery("newest", "lib-a")))).toBe(true);
    expect(stale(key(songsQuery("alphabeticalByName", "lib-a")))).toBe(true);
    expect(stale(key(artistsQuery("lib-a")))).toBe(true);
    // The whole catalogue holds this library's albums too.
    expect(stale(key(albumsQuery("newest", undefined)))).toBe(true);
    expect(stale(key(albumsQuery("newest", "lib-b")))).toBe(false);
    expect(stale(key(attentionSummaryQuery("lib-a")))).toBe(true);
    expect(stale(key(attentionSummaryQuery("lib-b")))).toBe(false);
  });

  it("marks the panels of the tracks it names and no other track's", () => {
    const stale = staleAfter("lib-a", [track("t1")]);
    expect(stale(key(trackPropertiesQuery("t1")))).toBe(true);
    expect(stale(key(trackEditQuery("t1")))).toBe(true);
    expect(stale(key(lyricsQuery("t1")))).toBe(true);
    expect(stale(key(trackPropertiesQuery("t2")))).toBe(false);
  });

  it("marks the pages a track or an album may appear on, which the feed does not name", () => {
    for (const event of [track("t1"), album("al1")]) {
      const stale = staleAfter("lib-a", [event]);
      expect(stale(key(albumQuery("any-album")))).toBe(true);
      expect(stale(key(artistQuery("any-artist")))).toBe(true);
      expect(stale(key(favoriteTracksQuery()))).toBe(true);
      expect(stale(key(smartPlaylistQuery("sp1")))).toBe(true);
      // The list of smart playlists is names and rules.
      expect(stale(key(smartPlaylistsQuery()))).toBe(false);
    }
  });

  it("marks only the artist an artist event names, since it is the portrait alone", () => {
    const stale = staleAfter("lib-a", [artist("ar1")]);
    expect(stale(key(artistQuery("ar1")))).toBe(true);
    expect(stale(key(artistQuery("ar2")))).toBe(false);
    expect(stale(key(albumQuery("any-album")))).toBe(false);
    expect(stale(key(artistsQuery("lib-a")))).toBe(true);
  });

  it("leaves a fresh draw and user state alone", () => {
    const stale = staleAfter("lib-a", [track("t1"), album("al1")]);
    expect(stale(key(randomSongsQuery("lib-a")))).toBe(false);
    expect(stale(key(playlistsQuery()))).toBe(false);
  });

  it("marks nothing for an empty page", () => {
    const stale = staleAfter("lib-a", []);
    expect(stale(key(albumsQuery("newest", "lib-a")))).toBe(false);
  });
});

describe("everythingIn", () => {
  it("marks every catalogue answer the library may be in, and stops at its edge", () => {
    const stale = everythingIn("lib-a");
    expect(stale(key(albumsQuery("newest", "lib-a")))).toBe(true);
    expect(stale(key(albumsQuery("newest", undefined)))).toBe(true);
    expect(stale(key(trackPropertiesQuery("any-track")))).toBe(true);
    expect(stale(key(artistQuery("any-artist")))).toBe(true);
    expect(stale(key(albumsQuery("newest", "lib-b")))).toBe(false);
    expect(stale(key(attentionSummaryQuery("lib-b")))).toBe(false);
    expect(stale(key(randomSongsQuery("lib-a")))).toBe(false);
  });
});
