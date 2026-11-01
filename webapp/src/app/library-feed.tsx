// The signed-in shell's follower of the library feeds.
import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";

import { listLibraries, listLibraryEvents } from "../api";
import { followLibraryFeeds } from "../lib/library-feed";
import { useLibraryScope } from "../state/library-scope";

/**
 * Keeps what this tab shows in step with the libraries' catalogues while it is
 * open: a scan, an upload from another device or a tag correction refreshes
 * the screens it concerns, without a reload. Renders nothing.
 *
 * Every library the account can see, not only the active one: the favourites
 * and the history list tracks from all of them.
 *
 * No acknowledgement is sent. RFC-007 decision 8 has a device acknowledge so
 * the operator can see what a purge will cost whom; a tab keeps nothing
 * between two loads and starts from the head every time, so a purge costs it
 * nothing, and acknowledging would count it among the devices a purge
 * strands. Nor are this tab's own changes filtered out by their originating
 * device: a second tab of the same browser shares that device and must still
 * hear them.
 */
export function LibraryFeed() {
  const client = useQueryClient();
  const { libraries } = useLibraryScope();
  useEffect(() => {
    if (libraries.length === 0) return;
    return followLibraryFeeds(libraries, {
      readPage: listLibraryEvents,
      readLibraries: listLibraries,
      invalidate: (predicate) => {
        void client.invalidateQueries({
          predicate: (query) => predicate(query.queryKey),
        });
      },
      hidden: () => document.visibilityState === "hidden",
      onVisibilityChange: (callback) => {
        document.addEventListener("visibilitychange", callback);
        return () => document.removeEventListener("visibilitychange", callback);
      },
    });
  }, [client, libraries]);
  return null;
}
