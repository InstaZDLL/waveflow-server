import { describe, expect, it } from "vitest";

import {
  advance,
  continuationOf,
  extendOrder,
  followQueue,
  orderForQueue,
  retreat,
  shuffledOrder,
} from "./player";

/**
 * The three ways a queue can be walked. These decide what plays next, so an
 * error here is silent until someone notices a track was skipped or the queue
 * stopped one short.
 */
describe("advance", () => {
  const line = [0, 1, 2, 3];

  it("walks forward and stops at the end when nothing repeats", () => {
    expect(advance(line, 0, "off", true)).toBe(1);
    expect(advance(line, 2, "off", true)).toBe(3);
    // The last track ends the queue rather than stepping past it.
    expect(advance(line, 3, "off", true)).toBeNull();
  });

  it("wraps to the front when the whole queue repeats", () => {
    expect(advance(line, 3, "all", true)).toBe(0);
    expect(advance(line, 1, "all", true)).toBe(2);
  });

  it("replays a track that ended, but lets next move past it", () => {
    // This is the only place `automatic` matters, and it is the difference
    // between repeat-one and a next button that appears broken.
    expect(advance(line, 2, "one", true)).toBe(2);
    expect(advance(line, 2, "one", false)).toBe(3);
    // Under repeat-one the last track still ends the queue when pressed past.
    expect(advance(line, 3, "one", false)).toBeNull();
  });

  it("follows the shuffled order rather than the queue's own", () => {
    const shuffled = [2, 0, 3, 1];
    expect(advance(shuffled, 2, "off", true)).toBe(0);
    expect(advance(shuffled, 3, "off", true)).toBe(1);
    expect(advance(shuffled, 1, "off", true)).toBeNull();
    expect(advance(shuffled, 1, "all", true)).toBe(2);
  });

  it("stops rather than guessing when the position is not in the order", () => {
    // A queue edited under the player can leave the index behind; guessing a
    // neighbour here would play something nobody chose.
    expect(advance(line, 9, "all", true)).toBeNull();
  });
});

describe("retreat", () => {
  const line = [0, 1, 2, 3];

  it("steps back and holds at the front", () => {
    expect(retreat(line, 2, "off")).toBe(1);
    expect(retreat(line, 0, "off")).toBeNull();
  });

  it("wraps to the end only when the whole queue repeats", () => {
    expect(retreat(line, 0, "all")).toBe(3);
    // Repeat-one is about one track, not about the queue's ends.
    expect(retreat(line, 0, "one")).toBeNull();
  });
});

describe("shuffledOrder", () => {
  it("leads with what is playing, so enabling it interrupts nothing", () => {
    const order = shuffledOrder(5, 3, () => 0.5);
    expect(order[0]).toBe(3);
  });

  it("is a permutation: every position once, none invented", () => {
    let seed = 0;
    const order = shuffledOrder(9, 4, () => {
      seed = (seed * 9301 + 49297) % 233280;
      return seed / 233280;
    });
    expect([...order].sort((a, b) => a - b)).toEqual([
      0, 1, 2, 3, 4, 5, 6, 7, 8,
    ]);
  });

  it("handles a queue with nothing selected yet", () => {
    // -1 is what an empty selection reads as; it must not lead the order.
    expect(shuffledOrder(3, -1, () => 0).sort()).toEqual([0, 1, 2]);
    expect(shuffledOrder(0, 0)).toEqual([]);
  });
});

describe("orderForQueue", () => {
  const linear = [0, 1, 2, 3, 4];

  it("is the queue's own order when shuffle is off", () => {
    expect(orderForQueue(5, 2, false, [4, 1, 0, 3, 2], false)).toEqual(linear);
  });

  it("draws afresh for a different queue of the same length", () => {
    // The defect this exists for: replacing one five-track album with another
    // left the previous draw standing. `advance` walks that draw, so a start
    // that fell at its end returned null and playback stopped after one track.
    const stale = [2, 0, 3, 1, 4];
    expect(advance(stale, 4, "off", true)).toBeNull();

    const fresh = orderForQueue(5, 4, true, stale, false, () => 0);
    expect(fresh[0]).toBe(4);
    expect(advance(fresh, 4, "off", true)).not.toBeNull();
    // Every position of the new queue is reachable, none invented.
    expect([...fresh].sort((a, b) => a - b)).toEqual(linear);
  });

  it("keeps the draw under way when the same queue grows", () => {
    const under = [2, 0, 3, 1, 4];
    // Adding a track mid-listen must not reshuffle what is left to hear.
    expect(orderForQueue(7, 2, true, under, true)).toEqual([
      2, 0, 3, 1, 4, 5, 6,
    ]);
  });

  it("drops positions a shortened queue no longer has", () => {
    expect(orderForQueue(3, 2, true, [2, 0, 3, 1, 4], true)).toEqual([2, 0, 1]);
  });

  it("draws for a queue that had no order yet", () => {
    expect(orderForQueue(3, 0, true, [], true, () => 0).sort()).toEqual([
      0, 1, 2,
    ]);
  });
});

describe("extendOrder", () => {
  it("keeps what survives and appends what is new, in queue order", () => {
    expect(extendOrder([3, 1, 0, 2], 6)).toEqual([3, 1, 0, 2, 4, 5]);
  });

  it("is a permutation of the new length whatever it was given", () => {
    expect([...extendOrder([9, 1], 4)].sort((a, b) => a - b)).toEqual([
      0, 1, 2, 3,
    ]);
  });
});

/**
 * Removing a track from the middle of a queue shifts everything after it down
 * by one, so an order made of positions stops meaning what it meant. Following
 * the numbers alone keeps playing — it just plays the wrong songs, quietly.
 */
describe("followQueue", () => {
  it("re-points the order at where the songs went", () => {
    // Queue [a, b, c, d, e] with b removed: c, d and e each shift down one.
    const moved = [0, -1, 1, 2, 3];
    // Was: e, b, a, d, c. b is gone; the rest keep their relative order.
    expect(followQueue([4, 1, 0, 3, 2], moved, 4)).toEqual([3, 0, 2, 1]);
  });

  it("appends positions the previous order never knew", () => {
    expect(followQueue([1, 0], [0, 1], 4)).toEqual([1, 0, 2, 3]);
  });

  it("is a permutation of the new queue whatever it is handed", () => {
    const order = followQueue([9, 2, 0], [3, -1, 1], 4);
    expect([...order].sort((a, b) => a - b)).toEqual([0, 1, 2, 3]);
  });

  it("has nothing to follow when every entry is gone", () => {
    expect(followQueue([0, 1], [-1, -1], 2)).toEqual([0, 1]);
  });
});

describe("orderForQueue, following a queue that lost a track", () => {
  it("keeps the draw when the removal is described", () => {
    // The wiring could not reach this before: it required the new queue to
    // begin with the whole of the old one, so any removal forced a reshuffle
    // and the rest of the listening was redrawn under the listener.
    const moved = [0, -1, 1, 2, 3];
    expect(orderForQueue(4, 0, true, [4, 1, 0, 3, 2], moved)).toEqual([
      3, 0, 2, 1,
    ]);
  });

  it("still draws afresh when nothing carried over", () => {
    const order = orderForQueue(3, 1, true, [2, 0, 1], false, () => 0);
    expect(order[0]).toBe(1);
  });
});

/**
 * The decision the player used to make inline, where no test could reach it —
 * which is how a branch of `orderForQueue` came to be covered by a unit test
 * while the wiring could never take it.
 */
describe("continuationOf", () => {
  const a = "a";
  const b = "b";
  const c = "c";
  const d = "d";

  it("is a fresh draw when there was no order yet", () => {
    expect(continuationOf(null, [a, b], true)).toBe(false);
  });

  it("is a fresh draw when the mode changed", () => {
    const drawn = { entries: [a, b], shuffle: false };
    expect(continuationOf(drawn, [a, b], true)).toBe(false);
  });

  it("is a fresh draw for a different queue of the same length", () => {
    // The defect that started this: length alone let one five-track album
    // inherit the draw made for another.
    const drawn = { entries: [a, b], shuffle: true };
    expect(continuationOf(drawn, [c, d], true)).toBe(false);
  });

  it("follows a removal from the middle", () => {
    // A prefix test refused this outright, so removing a track reshuffled
    // everything that was left to hear.
    const drawn = { entries: [a, b, c], shuffle: true };
    expect(continuationOf(drawn, [a, c], true)).toEqual([0, -1, 1]);
  });

  it("follows an append", () => {
    const drawn = { entries: [a, b], shuffle: true };
    expect(continuationOf(drawn, [a, b, c], true)).toEqual([0, 1]);
  });

  it("follows a reordering", () => {
    const drawn = { entries: [a, b, c], shuffle: true };
    expect(continuationOf(drawn, [c, a, b], true)).toEqual([1, 2, 0]);
  });
});
