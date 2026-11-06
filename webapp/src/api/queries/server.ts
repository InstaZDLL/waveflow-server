// The running server itself, as distinct from anything it holds.

import { type QueryClient, queryOptions } from "@tanstack/react-query";
import {
  ApiError,
  getServerIdentity,
  getTranscodeStatus,
  getTranscodingProfiles,
  getUpdateStatus,
} from "../index";

/**
 * The server's version and the source it offers.
 *
 * On the client's default freshness, not held for ever: the offer is of the
 * server's source, not the bundle's, and a server upgraded under an open page
 * runs the new version at once. Asked again, once stale, when the page is next
 * mounted or focused, which is when anyone reads it — not on a timer, which
 * would keep an idle sign-in page asking for a link nobody is looking at.
 *
 * Focus is switched back on here alone. The client turns it off so a library
 * left in the background does not answer a return with a round of requests;
 * this is one small request, and only the sign-in page and Settings hold it.
 *
 * Retried, unlike the client's other questions, on a transient failure alone
 * and at most twice: a 5xx, such as a proxy answering 503 while the server
 * restarts, or a request that never got an answer — `fetch` rejects those
 * with a `TypeError`. Either would otherwise leave the sign-in page without
 * the offer until it is next opened. A 4xx, or a body that is not the probe's
 * JSON, is an answer, and asking again would not change it.
 */
export const serverIdentityQuery = () =>
  queryOptions({
    queryKey: ["server-identity"],
    queryFn: getServerIdentity,
    retry: (failures, error) =>
      failures < 2 &&
      ((error instanceof ApiError && error.status >= 500) ||
        error instanceof TypeError),
    refetchOnWindowFocus: true,
  });

/**
 * How much transcoding the server is doing, against what it allows.
 *
 * Re-asked every ten seconds while the administration screen holds it: the
 * count moves as fast as listeners start and stop, and the answer is a few
 * integers read from memory. A hidden tab stops asking, which is TanStack
 * Query's default and kept here, and so does the screen once it is left.
 */
export const transcodeStatusQuery = () =>
  queryOptions({
    queryKey: ["transcode-status"],
    queryFn: getTranscodeStatus,
    refetchInterval: 10_000,
    staleTime: 0,
  });

/** The account's transcoding profile, and the client names seen streaming. */
export const transcodingProfilesQuery = () =>
  queryOptions({
    queryKey: ["transcoding-profiles"],
    queryFn: getTranscodingProfiles,
  });

/**
 * Whether a newer server is out, and whether the check may run (RFC-028).
 * Administrators only: the shell asks only for one.
 *
 * Held for an hour: the server reads its manifest once a day, so asking more
 * often would only repeat the same answer. Not retried, and not refetched on
 * focus: a dialog that reappears whenever the tab is focused is a nag.
 */
export const updateStatusQuery = () =>
  queryOptions({
    queryKey: ["server-update"],
    queryFn: getUpdateStatus,
    staleTime: 60 * 60 * 1000,
    retry: false,
    refetchOnWindowFocus: false,
  });

/**
 * After an administrator turns the check on: the server reads its manifest at
 * once, in the background, and the `PUT` answers before it has. Asked again a
 * little later — twice, the second past the check's own timeout — so a version
 * already out is shown in this session, from the dialog or from Settings alike.
 */
export function refreshUpdateStatusSoon(queryClient: QueryClient): void {
  for (const delay of [5_000, 20_000]) {
    window.setTimeout(() => {
      void queryClient.invalidateQueries({
        queryKey: updateStatusQuery().queryKey,
      });
    }, delay);
  }
}
