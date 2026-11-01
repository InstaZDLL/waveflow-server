import { describe, expect, it } from "vitest";

import {
  albumQuery,
  albumsQuery,
  artistAlbumsQuery,
  artistQuery,
  artistsQuery,
  artistTopSongsQuery,
  attentionSummaryQuery,
  canvasQuery,
  creditCandidatesQuery,
  creditDecisionsQuery,
  favoriteTracksQuery,
  filedTreeQuery,
  folderTracksQuery,
  genreSongsQuery,
  genresQuery,
  historyTracksQuery,
  lyricsQuery,
  mixesQuery,
  mixQuery,
  playlistsQuery,
  purgedTracksQuery,
  qualitySummaryQuery,
  qualityTracksQuery,
  randomSongsQuery,
  searchQuery,
  smartPlaylistQuery,
  smartPlaylistsQuery,
  songsQuery,
  trackEditQuery,
  trackPropertiesQuery,
  trackQualityQuery,
  unavailableTracksQuery,
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

  it("marks the daily mixes by their scope, which is not their last element", () => {
    const stale = staleAfter("lib-a", [track("t1")]);
    expect(stale(key(mixesQuery("lib-a", "2026-11-01")))).toBe(true);
    expect(stale(key(mixQuery("recent", "lib-a", "2026-11-01")))).toBe(true);
    expect(stale(key(mixesQuery("lib-b", "2026-11-01")))).toBe(false);
    expect(stale(key(mixQuery("recent", "lib-b", "2026-11-01")))).toBe(false);
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

/**
 * Every root the mapping knows, each built from its real query options: a root
 * dropped from a set, or a key reshaped so its scope moves, leaves an answer
 * stale on screen, and only a case per root makes that fail here.
 */
describe("every root a library change can make stale", () => {
  const DAY = "2026-11-01";
  const listings = (scope: string | undefined) => [
    albumsQuery("newest", scope),
    songsQuery("alphabeticalByName", scope),
    artistsQuery(scope),
    genresQuery(scope),
    genreSongsQuery("Ambient", scope),
    searchQuery("needle", scope),
    mixesQuery(scope, DAY),
    mixQuery("recent", scope, DAY),
  ];
  const panels = (libraryId: string) => [
    attentionSummaryQuery(libraryId),
    creditCandidatesQuery(libraryId),
    creditDecisionsQuery(libraryId),
    qualitySummaryQuery(libraryId),
    qualityTracksQuery(libraryId, {}),
    filedTreeQuery(libraryId),
    folderTracksQuery(libraryId, "folder-1"),
    unavailableTracksQuery(libraryId),
    purgedTracksQuery(libraryId),
  ];
  const pages = [
    albumQuery("al9"),
    artistQuery("ar9"),
    artistAlbumsQuery("ar9"),
    artistTopSongsQuery("ar9"),
    favoriteTracksQuery(),
    historyTracksQuery(50),
    smartPlaylistQuery("sp9"),
  ];
  const trackPanels = (trackId: string) => [
    trackPropertiesQuery(trackId),
    trackEditQuery(trackId),
    lyricsQuery(trackId),
    canvasQuery(trackId),
    trackQualityQuery(trackId),
  ];
  const expectAll = (
    stale: (queryKey: readonly unknown[]) => boolean,
    options: Array<{ queryKey: readonly unknown[] }>,
    expected: boolean,
  ) => {
    for (const option of options) {
      expect(stale(key(option)), JSON.stringify(key(option))).toBe(expected);
    }
  };

  it("a track event reaches each of them, and none of another library's", () => {
    const stale = staleAfter("lib-a", [track("t1")]);
    expectAll(stale, listings("lib-a"), true);
    expectAll(stale, listings(undefined), true);
    expectAll(stale, panels("lib-a"), true);
    expectAll(stale, pages, true);
    expectAll(stale, trackPanels("t1"), true);
    expectAll(stale, listings("lib-b"), false);
    expectAll(stale, panels("lib-b"), false);
    expectAll(stale, trackPanels("t2"), false);
  });

  it("an album event reaches the listings and the pages, not a track's panels", () => {
    const stale = staleAfter("lib-a", [album("al1")]);
    expectAll(stale, listings("lib-a"), true);
    expectAll(stale, panels("lib-a"), true);
    expectAll(stale, pages, true);
    expectAll(stale, trackPanels("t1"), false);
  });

  it("going back to the catalogue reaches each of them, any id, this library only", () => {
    const stale = everythingIn("lib-a");
    expectAll(stale, listings("lib-a"), true);
    expectAll(stale, listings(undefined), true);
    expectAll(stale, panels("lib-a"), true);
    expectAll(stale, pages, true);
    expectAll(stale, trackPanels("any-track"), true);
    expectAll(stale, listings("lib-b"), false);
    expectAll(stale, panels("lib-b"), false);
  });
});
