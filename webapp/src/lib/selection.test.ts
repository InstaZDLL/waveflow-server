import { describe, expect, it } from "vitest";

import { NOTHING_PICKED, pick, pickAll, pickedIn } from "./selection";

const ids = ["a", "b", "c", "d", "e"];
const names = (picked: { selected: ReadonlySet<string> }) =>
  [...picked.selected].sort();

describe("pick", () => {
  it("toggles a row and makes it the anchor", () => {
    const once = pick(ids, NOTHING_PICKED, 1, false);
    expect(names(once)).toEqual(["b"]);
    expect(once.anchor).toBe(1);
    expect(names(pick(ids, once, 1, false))).toEqual([]);
  });

  it("picks the range from the anchor with Shift, either way", () => {
    const anchored = pick(ids, NOTHING_PICKED, 1, false);
    expect(names(pick(ids, anchored, 3, true))).toEqual(["b", "c", "d"]);
    const backwards = pick(ids, NOTHING_PICKED, 3, false);
    expect(names(pick(ids, backwards, 0, true))).toEqual(["a", "b", "c", "d"]);
  });

  it("keeps the anchor, so a second Shift-click redraws from it", () => {
    let picked = pick(ids, NOTHING_PICKED, 2, false);
    picked = pick(ids, picked, 4, true);
    expect(picked.anchor).toBe(2);
    picked = pick(ids, picked, 0, true);
    // Added to what was picked: the range never unpicks a row.
    expect(names(picked)).toEqual(["a", "b", "c", "d", "e"]);
  });

  it("toggles with Shift when there is no anchor to range from", () => {
    expect(names(pick(ids, NOTHING_PICKED, 2, true))).toEqual(["c"]);
    // An anchor past the end of a list that shrank is no anchor.
    const stale = { selected: new Set<string>(), anchor: 9 };
    expect(names(pick(ids, stale, 1, true))).toEqual(["b"]);
  });

  it("ignores a position outside the list", () => {
    expect(pick(ids, NOTHING_PICKED, 7, false)).toBe(NOTHING_PICKED);
  });
});

describe("pickAll", () => {
  it("picks every row, then none", () => {
    const all = pickAll(ids, NOTHING_PICKED);
    expect(names(all)).toEqual(ids);
    expect(names(pickAll(ids, all))).toEqual([]);
  });

  it("picks every row when only some are", () => {
    const some = pick(ids, NOTHING_PICKED, 0, false);
    expect(names(pickAll(ids, some))).toEqual(ids);
  });
});

describe("pickedIn", () => {
  it("keeps the list's order and drops a row that left it", () => {
    const picked = { selected: new Set(["d", "gone", "a"]), anchor: null };
    expect(pickedIn(ids, picked)).toEqual(["a", "d"]);
  });
});
