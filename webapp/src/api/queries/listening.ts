// An account's listening: stats, mixes, favourites, history, scrobbles.

import { queryOptions } from "@tanstack/react-query";
import {
  playsByInstant,
  scrobbleRows,
  tracksToName,
} from "../../lib/scrobbling";
import {
  getListeningStats,
  getMix,
  getTrack,
  listFavorites,
  listHistory,
  listMixes,
  listNowPlaying,
  listScrobbleDestinations,
  listScrobbleLinks,
  listUncertainScrobbles,
  type StatsPeriod,
} from "../index";

const HISTORY_PLAYS = 200;

/**
 * One account's listening over a period (RFC-024 decision 6). The period is
 * the key, so switching it asks again rather than relabelling the last
 * answer, and so is the listener's day (`useLocalDay`): the buckets are
 * local days, and an answer from before midnight, or from another timezone,
 * cuts them in the wrong place.
 */
export const statsQuery = (period: StatsPeriod, day: string) =>
  queryOptions({
    queryKey: ["stats", period, day],
    queryFn: () => getListeningStats(period),
  });

/**
 * The mixes on the home page (RFC-024 decision 7). The server draws them once
 * a day, so the answer is held for a while rather than re-asked on every
 * visit to the albums — and the listener's day is in the key, because a key
 * that never changes is never asked again: past midnight the shelf would keep
 * yesterday's draw for as long as the tab stayed open.
 */
export const mixesQuery = (scope: string | undefined, day: string) =>
  queryOptions({
    queryKey: ["mixes", scope ?? null, day],
    queryFn: () => listMixes(scope),
    staleTime: 5 * 60_000,
  });

export const mixQuery = (
  mixId: string,
  scope: string | undefined,
  day: string,
) =>
  queryOptions({
    queryKey: ["mix", mixId, scope ?? null, day],
    queryFn: () => getMix(mixId, scope),
    staleTime: 5 * 60_000,
  });

/**
 * Who is listening, re-asked on a timer.
 *
 * The panel used to hold the previous reading in a `useState` of its own,
 * because the hook beneath it blanked its value at the start of every poll and
 * flashed "nobody is listening" thirty seconds apart. A query keeps what it has
 * while it refetches, so that workaround is gone rather than moved.
 */
export const nowPlayingQuery = () =>
  queryOptions({
    queryKey: ["now-playing"],
    queryFn: listNowPlaying,
    refetchInterval: 30_000,
    staleTime: 0,
  });

/**
 * The favourites, resolved to the tracks themselves.
 *
 * `Promise.allSettled`, so one track that has since become unreadable costs its
 * own row and not the page.
 */
export const favoriteTracksQuery = () =>
  queryOptions({
    queryKey: ["favorite-tracks"],
    queryFn: async () => {
      const favorites = await listFavorites();
      const tracks = favorites.filter((item) => item.entity_type === "track");
      const resolved = await Promise.allSettled(
        tracks.map((item) => getTrack(item.entity_id)),
      );
      return resolved.flatMap((result) =>
        result.status === "fulfilled" ? [result.value] : [],
      );
    },
  });

/**
 * Recently played, one entry per track.
 *
 * The route answers plays, not songs, and the same track appears many times.
 * Keeping the first sighting of each id gives "recently played" in the order it
 * was last played, and asks for each track once.
 */
export const historyTracksQuery = (limit: number) =>
  queryOptions({
    queryKey: ["history-tracks", limit],
    queryFn: async () => {
      const plays = await listHistory(HISTORY_PLAYS);
      const seen = new Set<string>();
      const ordered = plays.filter((play) => {
        if (seen.has(play.track_id)) return false;
        seen.add(play.track_id);
        return true;
      });
      const resolved = await Promise.allSettled(
        ordered.slice(0, limit).map((play) => getTrack(play.track_id)),
      );
      return resolved.flatMap((result) =>
        result.status === "fulfilled" ? [result.value] : [],
      );
    },
  });

export const scrobbleRowsQuery = () =>
  queryOptions({
    queryKey: ["scrobble-rows"],
    queryFn: async () => {
      const [destinations, links] = await Promise.all([
        listScrobbleDestinations(),
        listScrobbleLinks(),
      ]);
      return scrobbleRows(destinations, links);
    },
  });

/**
 * The listens nobody has answered yet, with the tracks behind them named.
 *
 * The history read happens only when there is something to name: an empty list
 * must not cost one on every visit to the page.
 */
export const uncertainScrobblesQuery = () =>
  queryOptions({
    queryKey: ["uncertain-scrobbles"],
    queryFn: async () => {
      const entries = await listUncertainScrobbles();
      if (entries.length === 0) {
        return { entries, titles: new Map<string, string>() };
      }
      const plays = playsByInstant(await listHistory(HISTORY_PLAYS));
      const resolved = await Promise.allSettled(
        tracksToName(entries, plays).map((id) => getTrack(id)),
      );
      const byTrack = new Map<string, string>();
      for (const result of resolved) {
        if (result.status === "fulfilled") {
          byTrack.set(result.value.id, result.value.title);
        }
      }
      // Re-keyed by entry, not by track: the row shows the title beside the
      // listen it belongs to, and two entries can name the same track.
      const titles = new Map<string, string>();
      for (const entry of entries) {
        const track = plays.get(entry.played_at);
        const title = track ? byTrack.get(track) : undefined;
        if (title) titles.set(entry.id, title);
      }
      return { entries, titles };
    },
  });
