import { describe, expect, it } from "vitest";

import { NOTHING_PICKED, pick, pickAll, pickedIn, unpick } from "./selection";

const ids = ["a", "b", "c", "d", "e"];
const names = (picked: { selected: ReadonlySet<string> }) =>
  [...picked.selected].sort();

describe("pick", () => {
  it("toggles a row and makes it the anchor", () => {
    const once = pick(ids, NOTHING_PICKED, 1, false);
    expect(names(once)).toEqual(["b"]);
    expect(once.anchor).toBe("b");
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
    expect(picked.anchor).toBe("c");
    picked = pick(ids, picked, 0, true);
    // Added to what was picked: the range never unpicks a row.
    expect(names(picked)).toEqual(["a", "b", "c", "d", "e"]);
  });

  it("toggles with Shift when there is no anchor to range from", () => {
    expect(names(pick(ids, NOTHING_PICKED, 2, true))).toEqual(["c"]);
    // An anchor whose row left the list is no anchor.
    const stale = { selected: new Set<string>(), anchor: "gone" };
    expect(names(pick(ids, stale, 1, true))).toEqual(["b"]);
  });

  it("follows the anchor's row, not its old position, in a re-read list", () => {
    // "b" is picked, then the list is read again without it: the row now at
    // position 1 is "c", which was never the anchor.
    const anchored = pick(["a", "b", "c", "d"], NOTHING_PICKED, 1, false);
    const shrunk = ["a", "c", "d"];
    expect(names(pick(shrunk, anchored, 2, true))).toEqual(["b", "d"]);
    // And where the anchor's row still is, the range starts from it.
    const moved = ["x", "a", "b", "c", "d"];
    expect(names(pick(moved, anchored, 4, true))).toEqual(["b", "c", "d"]);
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

describe("unpick", () => {
  it("takes away what an action took, and keeps what was picked since", () => {
    const picked = { selected: new Set(["a", "b", "e"]), anchor: "e" };
    const left = unpick(picked, ["a", "b"]);
    expect(names(left)).toEqual(["e"]);
    expect(left.anchor).toBe("e");
  });
});

describe("pickedIn", () => {
  it("keeps the list's order and drops a row that left it", () => {
    const picked = { selected: new Set(["d", "gone", "a"]), anchor: null };
    expect(pickedIn(ids, picked)).toEqual(["a", "d"]);
  });
});
