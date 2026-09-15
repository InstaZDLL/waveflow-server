import { QueryClient } from "@tanstack/react-query";

/**
 * The cache that sits between two pages.
 *
 * Every screen used to fetch on mount and blank itself first: `useAsync` set
 * its value to `null` before each run, so leaving a page and coming back drew
 * the skeleton again even when the answer arrived in nine milliseconds. Measured
 * on a real library over a local network, that was 79ms of flicker per
 * navigation — brief, and visible on every single one.
 *
 * A cache removes the cause rather than hiding it. A page already visited
 * renders from what is held, synchronously, with no empty state to pass
 * through; a refetch happens behind the content already on screen.
 *
 * There is no server rendering here — the binary serves an empty document and
 * React builds everything in the browser — so none of this depends on
 * hydration, and the same code would run unchanged inside a desktop shell.
 */
export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        /**
         * How long an answer is served without asking again.
         *
         * Thirty seconds, not zero: a catalogue changes when somebody scans,
         * which is rare, and the cost of being briefly behind is a cover that
         * appears one navigation late. Zero would refetch on every mount and
         * put the request back that this exists to remove — the flicker would
         * go, but the traffic would not.
         */
        staleTime: 30_000,
        /**
         * How long an unused answer is kept before it is dropped.
         *
         * Five minutes, so wandering through the catalogue and coming back
         * still costs nothing. Past that the memory is worth more than the
         * saved request, and the artwork object URLs are held elsewhere.
         */
        gcTime: 5 * 60_000,
        /**
         * `call()` already renews an expired session and retries once, so a
         * failure that reaches here is a real one. Retrying it would turn a
         * 404 into four 404s and delay the message a person is waiting for.
         */
        retry: false,
        /**
         * Refetching when a window regains focus is right for a dashboard
         * somebody leaves open. This is a music library: the tab is in the
         * background while the music plays, and coming back to it should show
         * what it showed, not start a round of requests.
         */
        refetchOnWindowFocus: false,
      },
    },
  });
}
