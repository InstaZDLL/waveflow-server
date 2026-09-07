import { describe, expect, it } from "vitest";

import { advance, retreat, shuffledOrder } from "./player";

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
