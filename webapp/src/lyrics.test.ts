import { describe, expect, it } from "vitest";

import { currentLyricLine } from "./pages";

/**
 * The highlight in the now-playing view. Off-by-one here shows a line before
 * it is sung or holds the previous one through it, which is the whole of what
 * a synced sheet is for.
 */
describe("currentLyricLine", () => {
  const sheet = [
    { start: 0, value: "one" },
    { start: 5_000, value: "two" },
    { start: 12_500, value: "three" },
  ];

  it("holds a line until the next one starts", () => {
    expect(currentLyricLine(sheet, 0)).toBe(0);
    expect(currentLyricLine(sheet, 4.999)).toBe(0);
    expect(currentLyricLine(sheet, 5)).toBe(1);
    expect(currentLyricLine(sheet, 12.4)).toBe(1);
    expect(currentLyricLine(sheet, 12.5)).toBe(2);
    // Past the last start it stays on the last line rather than falling off.
    expect(currentLyricLine(sheet, 600)).toBe(2);
  });

  it("highlights nothing before the first line", () => {
    // A sheet whose first line starts late leaves an intro unhighlighted.
    const late = [{ start: 3_000, value: "late" }];
    expect(currentLyricLine(late, 0)).toBe(-1);
    expect(currentLyricLine(late, 2.9)).toBe(-1);
    expect(currentLyricLine(late, 3)).toBe(0);
  });

  it("highlights nothing in a sheet that carries no times", () => {
    // An unsynced sheet is a list of lines with no `start` at all. Every
    // position has to answer -1, or a plain lyric sheet would follow a
    // timeline it does not have.
    const plain = [{ value: "a" }, { value: "b" }];
    expect(currentLyricLine(plain, 0)).toBe(-1);
    expect(currentLyricLine(plain, 90)).toBe(-1);
  });

  it("stops at the first untimed line rather than skipping it", () => {
    // LRC files exist with a stray untimed line; treating it as "no start, keep
    // looking" would let a later line light up while this one is on screen.
    const mixed = [
      { start: 0, value: "a" },
      { value: "b" },
      { start: 9_000, value: "c" },
    ];
    expect(currentLyricLine(mixed, 30)).toBe(0);
  });

  it("has nothing to highlight in an empty sheet", () => {
    expect(currentLyricLine([], 10)).toBe(-1);
  });
});
