import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { LibraryEventPage } from "../api";
import {
  type FeedDependencies,
  followLibraryFeeds,
  HIDDEN_GRACE_MS,
  MAX_BACKOFF_MS,
  nextDelay,
  POLL_MS,
} from "./library-feed";

const page = (
  nextCursor: number,
  events: Array<{ entity_type: "track" | "album"; entity_id: string }> = [],
  hasMore = false,
): LibraryEventPage => ({
  events: events.map((event, index) => ({
    ...event,
    cursor: nextCursor - events.length + index + 1,
    action: "upsert",
    changed_at: 0,
  })),
  next_cursor: nextCursor,
  has_more: hasMore,
  purged_through: 0,
});

const refused = (status: number) =>
  Object.assign(new Error("refused"), { status });

/**
 * A feed whose answers the test queues, a document whose visibility it flips,
 * and a record of every read and invalidation.
 */
function harness() {
  const answers: Array<() => Promise<LibraryEventPage>> = [];
  const reads: Array<[string, number]> = [];
  const invalidated: Array<(key: readonly unknown[]) => boolean> = [];
  let hidden = false;
  let notify = () => {};
  const heads = new Map([["lib-a", 40]]);
  const deps: FeedDependencies = {
    readPage: (libraryId, after) => {
      reads.push([libraryId, after]);
      const next = answers.shift();
      return next ? next() : Promise.resolve(page(after));
    },
    readLibraries: vi.fn(async () =>
      [...heads].map(([id, events_cursor]) => ({ id, events_cursor })),
    ),
    invalidate: (predicate) => invalidated.push(predicate),
    hidden: () => hidden,
    onVisibilityChange: (callback) => {
      notify = callback;
      return () => {
        notify = () => {};
      };
    },
  };
  return {
    deps,
    answers,
    reads,
    invalidated,
    heads,
    setHidden(value: boolean) {
      hidden = value;
      notify();
    },
  };
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("followLibraryFeeds", () => {
  it("asks from the listed head, then from where each answer ended", async () => {
    const feed = harness();
    feed.answers.push(async () =>
      page(12, [{ entity_type: "track", entity_id: "t1" }]),
    );
    const stop = followLibraryFeeds(
      [{ id: "lib-a", events_cursor: 10 }],
      feed.deps,
    );
    expect(feed.reads).toEqual([]);
    await vi.advanceTimersByTimeAsync(POLL_MS);
    await vi.advanceTimersByTimeAsync(POLL_MS);
    expect(feed.reads).toEqual([
      ["lib-a", 10],
      ["lib-a", 12],
    ]);
    // One page with events, one without: only the first invalidates.
    expect(feed.invalidated).toHaveLength(1);
    expect(feed.invalidated[0]?.(["track-properties", "t1"])).toBe(true);
    stop();
  });

  it("goes back to the catalogue once when the cursor is older than retention", async () => {
    const feed = harness();
    feed.answers.push(() => Promise.reject(refused(409)));
    const stop = followLibraryFeeds(
      [{ id: "lib-a", events_cursor: 3 }],
      feed.deps,
    );
    await vi.advanceTimersByTimeAsync(POLL_MS);
    expect(feed.deps.readLibraries).toHaveBeenCalledTimes(1);
    expect(feed.invalidated).toHaveLength(1);
    // Everything, not one track: the events that were cut cannot be named.
    expect(feed.invalidated[0]?.(["track-properties", "any"])).toBe(true);
    await vi.advanceTimersByTimeAsync(POLL_MS);
    // And it resumes from the head the list now names, not from where it was.
    expect(feed.reads.at(-1)).toEqual(["lib-a", 40]);
    expect(feed.invalidated).toHaveLength(1);
    stop();
  });

  it("re-reads the screens instead of walking a gap of more than a page", async () => {
    const feed = harness();
    feed.answers.push(async () =>
      page(510, [{ entity_type: "track", entity_id: "t1" }], true),
    );
    const stop = followLibraryFeeds(
      [{ id: "lib-a", events_cursor: 10 }],
      feed.deps,
    );
    await vi.advanceTimersByTimeAsync(POLL_MS);
    expect(feed.reads).toEqual([["lib-a", 10]]);
    expect(feed.invalidated).toHaveLength(1);
    expect(feed.invalidated[0]?.(["album", "any"])).toBe(true);
    await vi.advanceTimersByTimeAsync(POLL_MS);
    expect(feed.reads.at(-1)).toEqual(["lib-a", 40]);
    stop();
  });

  it("stops asking for a library the account can no longer see", async () => {
    const feed = harness();
    feed.answers.push(() => Promise.reject(refused(404)));
    const stop = followLibraryFeeds(
      [
        { id: "lib-a", events_cursor: 1 },
        { id: "lib-b", events_cursor: 2 },
      ],
      feed.deps,
    );
    await vi.advanceTimersByTimeAsync(POLL_MS);
    await vi.advanceTimersByTimeAsync(POLL_MS);
    expect(feed.reads).toEqual([
      ["lib-a", 1],
      ["lib-b", 2],
      ["lib-b", 2],
    ]);
    stop();
  });

  it("backs off while the server fails, and returns to the interval once it answers", async () => {
    const feed = harness();
    const down = () => Promise.reject(new TypeError("network"));
    feed.answers.push(down, down);
    const stop = followLibraryFeeds(
      [{ id: "lib-a", events_cursor: 5 }],
      feed.deps,
    );
    await vi.advanceTimersByTimeAsync(POLL_MS);
    expect(feed.reads).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(nextDelay(1) - 1);
    expect(feed.reads).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(feed.reads).toHaveLength(2);
    await vi.advanceTimersByTimeAsync(nextDelay(2));
    expect(feed.reads).toHaveLength(3);
    // Answered: the next ask is one interval away again, from the same cursor.
    await vi.advanceTimersByTimeAsync(POLL_MS);
    expect(feed.reads).toHaveLength(4);
    expect(feed.reads.every(([, after]) => after === 5)).toBe(true);
    stop();
  });

  it("stops asking once hidden for long, and resumes from its cursor on return", async () => {
    const feed = harness();
    feed.answers.push(async () =>
      page(9, [{ entity_type: "album", entity_id: "al1" }]),
    );
    const stop = followLibraryFeeds(
      [{ id: "lib-a", events_cursor: 7 }],
      feed.deps,
    );
    await vi.advanceTimersByTimeAsync(POLL_MS);
    expect(feed.reads).toEqual([["lib-a", 7]]);

    feed.setHidden(true);
    await vi.advanceTimersByTimeAsync(HIDDEN_GRACE_MS);
    const whileHidden = feed.reads.length;
    await vi.advanceTimersByTimeAsync(10 * POLL_MS);
    expect(feed.reads).toHaveLength(whileHidden);

    feed.setHidden(false);
    await vi.advanceTimersByTimeAsync(0);
    // At once, not one interval later, and from where the last answer ended.
    expect(feed.reads).toHaveLength(whileHidden + 1);
    expect(feed.reads.at(-1)).toEqual(["lib-a", 9]);
    stop();
  });

  it("keeps asking through a short absence", async () => {
    const feed = harness();
    const stop = followLibraryFeeds(
      [{ id: "lib-a", events_cursor: 7 }],
      feed.deps,
    );
    feed.setHidden(true);
    await vi.advanceTimersByTimeAsync(HIDDEN_GRACE_MS - 1);
    feed.setHidden(false);
    await vi.advanceTimersByTimeAsync(10 * POLL_MS);
    expect(feed.reads.length).toBeGreaterThanOrEqual(10);
    stop();
  });

  it("asks nothing after it is stopped", async () => {
    const feed = harness();
    const stop = followLibraryFeeds(
      [{ id: "lib-a", events_cursor: 7 }],
      feed.deps,
    );
    stop();
    await vi.advanceTimersByTimeAsync(10 * POLL_MS);
    expect(feed.reads).toEqual([]);
  });
});

describe("nextDelay", () => {
  it("doubles from the interval and stops at its ceiling", () => {
    expect(nextDelay(0)).toBe(POLL_MS);
    expect(nextDelay(1)).toBe(2 * POLL_MS);
    expect(nextDelay(2)).toBe(4 * POLL_MS);
    expect(nextDelay(20)).toBe(MAX_BACKOFF_MS);
  });
});
