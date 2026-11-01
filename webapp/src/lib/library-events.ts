/**
 * Which cached answers a change in a library's catalogue makes stale.
 *
 * The library feed (RFC-007) names an entity and nothing about where it
 * appears: a track's event does not say which album lists it, an album's does
 * not say which artist's page shows it. So the answer here is a predicate over
 * query keys, deliberately generous where the feed is silent — invalidating
 * marks an answer stale and refetches only what is on screen, so a key wrongly
 * included costs one request, and a key wrongly left out is a list that stays
 * wrong until a reload.
 *
 * The keys are the ones `api/queries/` writes; a key added there that shows
 * catalogue data belongs in one of the sets below.
 */

import type { LibraryEvent } from "../api";

type QueryKey = readonly unknown[];

/**
 * The catalogue's listings, with where each key holds its scope: one library's
 * id, or `null` for the whole catalogue, which a change in any library moves.
 *
 * The daily mixes are here because they are drawn from a seed of the account
 * and the day: asked again, they keep their order and gain what was added.
 * `random-songs` is not: a fresh draw is what that screen promises, and a scan
 * redrawing it under the listener every few seconds would be the feed working
 * and the page failing.
 */
const SCOPED_LISTINGS = new Map([
  ["albums", 2],
  ["songs", 2],
  ["artists", 1],
  ["genres", 1],
  ["genre-songs", 2],
  ["search", 2],
  ["mixes", 1],
  ["mix", 2],
]);

/** The maintenance panels, each keyed with its library second. */
const LIBRARY_PANELS = new Set([
  "attention",
  "quality",
  "missing",
  "filed-tree",
  "folder-tracks",
]);

/**
 * Pages that show tracks or albums but are keyed by something the feed does
 * not name — an album page lists its tracks, an artist page its albums and
 * counts, the favourites and history their rows. A track or album event may
 * concern any of them, so all are marked.
 */
const CONTENT_PAGES = new Set([
  "album",
  "artist",
  "artist-albums",
  "artist-top-songs",
  "favorite-tracks",
  "history-tracks",
]);

/** One track's panels, keyed by its id second. */
const TRACK_PANELS = new Set([
  "track-properties",
  "track-edit",
  "lyrics",
  "canvas",
  "track-quality",
]);

/**
 * The cached answers a page of one library's events makes stale.
 *
 * Every event moves the library's listings and panels: an added track changes
 * the songs, a retag may change an album's year, a deleted one its count. What
 * an event adds on top depends on what it names.
 */
export function staleAfter(
  libraryId: string,
  events: readonly Pick<LibraryEvent, "entity_type" | "entity_id">[],
): (queryKey: QueryKey) => boolean {
  const tracks = new Set<string>();
  const artists = new Set<string>();
  let content = false;
  for (const event of events) {
    if (event.entity_type === "track") tracks.add(event.entity_id);
    if (event.entity_type === "artist") artists.add(event.entity_id);
    // An artist's event is its portrait alone (RFC-007 decision 6, amended):
    // its counts are derived and never announced, so it moves no listing of
    // tracks or albums.
    if (event.entity_type !== "artist") content = true;
  }
  if (events.length === 0) return () => false;
  return (queryKey) => {
    const [root, second] = queryKey;
    if (typeof root !== "string") return false;
    if (SCOPED_LISTINGS.has(root)) return inScope(queryKey, libraryId);
    if (LIBRARY_PANELS.has(root)) return second === libraryId;
    if (TRACK_PANELS.has(root)) return tracks.has(second as string);
    if (content && CONTENT_PAGES.has(root)) return true;
    // A smart playlist's tracks are a query over the catalogue; the list of
    // them is names and rules, which no catalogue change moves.
    if (content && root === "smart-playlists") return queryKey.length > 1;
    if (root === "artist") return artists.has(second as string);
    return false;
  };
}

/**
 * Everything one library's catalogue may have changed, for when the client no
 * longer knows what did: its cursor fell behind what retention kept, or it is
 * so far behind that reading the gap would cost more than re-reading the
 * screen.
 */
export function everythingIn(
  libraryId: string,
): (queryKey: QueryKey) => boolean {
  return (queryKey) => {
    const [root, second] = queryKey;
    if (typeof root !== "string") return false;
    if (SCOPED_LISTINGS.has(root)) return inScope(queryKey, libraryId);
    if (LIBRARY_PANELS.has(root)) return second === libraryId;
    if (root === "smart-playlists") return queryKey.length > 1;
    return CONTENT_PAGES.has(root) || TRACK_PANELS.has(root);
  };
}

/** Whether a listing's scope covers the library. */
function inScope(queryKey: QueryKey, libraryId: string): boolean {
  const scope = queryKey[SCOPED_LISTINGS.get(queryKey[0] as string) ?? 0];
  return scope === null || scope === libraryId;
}
