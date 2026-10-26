// One track's panels: its properties, words, canvas and tag editor.

import { queryOptions } from "@tanstack/react-query";
import {
  canvasUrl,
  getLyrics,
  getTrackCredits,
  getTrackOverrides,
  getTrackProperties,
} from "../index";

/**
 * One track as its properties panel shows it, with the corrections behind
 * its tags so each corrected value can say that it is one.
 *
 * Not `trackEditQuery`, though it reads the same two routes: that one is the
 * editor's and is invalidated by its saves; sharing a key would tie the two
 * screens' lifetimes together for no saving worth having.
 */
export const trackPropertiesQuery = (trackId: string) =>
  queryOptions({
    queryKey: ["track-properties", trackId],
    queryFn: async () => {
      // The corrections only mark which tags were corrected. A failure to
      // read them costs those marks, not the panel.
      const [track, tracked] = await Promise.all([
        getTrackProperties(trackId),
        getTrackOverrides(trackId).catch(() => null),
      ]);
      return { track, tracked };
    },
  });

export const lyricsQuery = (trackId: string) =>
  queryOptions({
    queryKey: ["lyrics", trackId],
    queryFn: () => getLyrics(trackId),
  });

/**
 * A canvas ticket, held for exactly as long as it is being looked at.
 *
 * `staleTime` and `gcTime` at zero, against the defaults, because what this
 * answers is not data but a **credential with a deadline** — an AEAD-sealed
 * ticket the `<video>` plays from, since it can send no Authorization header.
 * A cache has no notion of that deadline and would go on serving the ticket
 * after it passed. The lifetime is `WAVEFLOW_STREAM_TICKET_TTL`, an hour by
 * default and an operator's to shorten, so "shorter than the cache" is a
 * configuration away rather than impossible. `player.tsx` checks the deadline
 * before playing a stream ticket for the same reason; here there is simply
 * nothing worth keeping.
 *
 * A track with no canvas answers `null`, which a query holds as an answer. The
 * hook this replaced could not tell that from "still waiting" and had to be
 * handed an object to unwrap.
 */
export const canvasQuery = (trackId: string) =>
  queryOptions({
    queryKey: ["canvas", trackId],
    queryFn: () => canvasUrl(trackId),
    staleTime: 0,
    gcTime: 0,
  });

/**
 * Everything the tag editor shows for one track: its effective values, and the
 * corrections behind them.
 *
 * Two reads rather than one, because provenance appears on this screen and
 * nowhere else — the catalogue shows the effective value everywhere.
 */
export const trackEditQuery = (trackId: string) =>
  queryOptions({
    queryKey: ["track-edit", trackId],
    queryFn: async () => {
      const [song, tracked] = await Promise.all([
        getTrackCredits(trackId),
        getTrackOverrides(trackId),
      ]);
      return { song, tracked };
    },
  });
