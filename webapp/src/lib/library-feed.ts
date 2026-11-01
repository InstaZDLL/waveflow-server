/**
 * Following the library feeds while a tab is open (RFC-007).
 *
 * The feed is a page of JSON read from a cursor, not a stream: the server
 * holds no connection and pushes nothing, so following it means asking again.
 * One request per library every {@link POLL_MS}, from the cursor the last
 * answer ended on, which is the whole of what the desktop does too.
 *
 * No React here, and no `fetch`: the reads and the invalidation are handed in,
 * so the timing — the pause while hidden, the backoff, the resync — is tested
 * with a fake clock rather than by waiting on one.
 */

import type { Library, LibraryEventPage } from "../api";
import { everythingIn, staleAfter } from "./library-events";

type Predicate = (queryKey: readonly unknown[]) => boolean;

/** How often an open tab asks each library what changed. */
export const POLL_MS = 15_000;

/**
 * How long a hidden tab keeps asking before it stops.
 *
 * Not zero: a listener who glances at another tab and comes back should find
 * the list already right, not a round of requests starting. Past a minute the
 * tab is a player in the background, and asking every fifteen seconds for a
 * screen nobody is looking at is traffic for nothing.
 */
export const HIDDEN_GRACE_MS = 60_000;

/** The longest a failing feed waits before asking again. */
export const MAX_BACKOFF_MS = 5 * 60_000;

/** The pause before the next ask, after `failures` failed ones in a row. */
export function nextDelay(failures: number): number {
  if (failures <= 0) return POLL_MS;
  return Math.min(POLL_MS * 2 ** failures, MAX_BACKOFF_MS);
}

export type FeedDependencies = {
  readPage: (
    libraryId: string,
    after: number,
    signal: AbortSignal,
  ) => Promise<LibraryEventPage>;
  /** The libraries as the list answers them now, with their heads. */
  readLibraries: () => Promise<Pick<Library, "id" | "events_cursor">[]>;
  invalidate: (predicate: Predicate) => void;
  /** Whether the document is hidden now. */
  hidden: () => boolean;
  /** Calls back on every change of visibility; answers how to stop. */
  onVisibilityChange: (callback: () => void) => () => void;
};

/**
 * Follows each library from the head it was listed with. Answers how to stop.
 *
 * The head is read before the catalogue is: the shell waits for the library
 * list before any catalogue screen asks, so the worst a change landing in
 * between does is arrive twice — once in the answer, once as an event — and
 * never not at all.
 */
export function followLibraryFeeds(
  libraries: readonly Pick<Library, "id" | "events_cursor">[],
  deps: FeedDependencies,
): () => void {
  const cursors = new Map(
    libraries.map((library) => [library.id, library.events_cursor]),
  );
  let failures = 0;
  let stopped = false;
  let paused = false;
  let reading: AbortController | null = null;
  let pollTimer: ReturnType<typeof setTimeout> | undefined;
  let graceTimer: ReturnType<typeof setTimeout> | undefined;

  const schedule = (delay: number) => {
    clearTimeout(pollTimer);
    pollTimer = setTimeout(() => void round(), delay);
  };

  /**
   * Sends the client back to the catalogue for one library: reads where its
   * feed stands now, then marks everything it shows stale. In that order, so
   * whatever the refetch answers is at least as new as the cursor kept.
   */
  const resync = async (libraryId: string) => {
    const listed = await deps.readLibraries();
    const head = listed.find((library) => library.id === libraryId);
    if (head) cursors.set(libraryId, head.events_cursor);
    else cursors.delete(libraryId);
    deps.invalidate(everythingIn(libraryId));
  };

  const readOne = async (
    libraryId: string,
    after: number,
    signal: AbortSignal,
  ) => {
    try {
      const page = await deps.readPage(libraryId, after, signal);
      if (signal.aborted || cursors.get(libraryId) !== after) return;
      if (page.has_more) {
        // More than a page behind: a scan of thousands of files, or a tab
        // that slept through one. Walking the gap a page at a time would
        // invalidate the same screens once per page, so the screens are
        // re-read once instead and the walk skipped.
        await resync(libraryId);
        return;
      }
      cursors.set(libraryId, page.next_cursor);
      if (page.events.length > 0) {
        deps.invalidate(staleAfter(libraryId, page.events));
      }
    } catch (error) {
      if (signal.aborted) return;
      const status = (error as { status?: unknown }).status;
      if (status === 409) {
        // The cursor is older than what retention kept: the events in the gap
        // are gone, and the surviving tail would look like a catch-up while
        // skipping them. The catalogue is the only way back.
        await resync(libraryId);
      } else if (status === 404) {
        // Deleted, or this account's membership withdrawn: the feed answers
        // exactly as it would for a library that never existed, and asking
        // again will not change that.
        cursors.delete(libraryId);
      } else {
        throw error;
      }
    }
  };

  const round = async () => {
    if (stopped || paused || reading) return;
    const controller = new AbortController();
    reading = controller;
    let failed = false;
    for (const [libraryId, after] of [...cursors]) {
      try {
        await readOne(libraryId, after, controller.signal);
      } catch {
        // The network, or a server answering 5xx. The other libraries are
        // still asked; the whole round backs off together, since one server
        // answers for all of them.
        failed = true;
      }
      if (controller.signal.aborted) break;
    }
    if (reading === controller) reading = null;
    if (controller.signal.aborted || stopped || paused) return;
    failures = failed ? failures + 1 : 0;
    if (cursors.size > 0) schedule(nextDelay(failures));
  };

  const pause = () => {
    paused = true;
    clearTimeout(pollTimer);
    reading?.abort();
    reading = null;
  };

  const visibilityChanged = () => {
    if (deps.hidden()) {
      clearTimeout(graceTimer);
      graceTimer = setTimeout(pause, HIDDEN_GRACE_MS);
      return;
    }
    clearTimeout(graceTimer);
    if (!paused) return;
    // Back from a long absence: ask now, from where the last answer ended,
    // rather than leave the screen stale for another interval.
    paused = false;
    failures = 0;
    void round();
  };

  const unsubscribe = deps.onVisibilityChange(visibilityChanged);
  if (deps.hidden()) visibilityChanged();
  if (cursors.size > 0) schedule(POLL_MS);

  return () => {
    stopped = true;
    unsubscribe();
    clearTimeout(pollTimer);
    clearTimeout(graceTimer);
    reading?.abort();
  };
}
