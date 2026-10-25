import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { mixesQuery, mixQuery, statsQuery } from "../api/queries";
import {
  localDayKey,
  msUntilNextLocalMidnight,
  OFFSET_CHECK_MS,
  watchLocalDay,
} from "./local-day";

/** 25 October 2026, half a minute before local midnight. */
const LATE = new Date(2026, 9, 25, 23, 59, 30);

describe("the listener's day", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(LATE);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  it("is the local date and the offset from UTC", () => {
    const offset = -LATE.getTimezoneOffset();
    expect(localDayKey(LATE)).toBe(`2026-10-25@${offset}`);
    expect(msUntilNextLocalMidnight(LATE)).toBe(30_000);
  });

  it("changes at midnight, and keeps changing every midnight after", () => {
    const seen: string[] = [];
    const stop = watchLocalDay((key) => seen.push(key));

    vi.advanceTimersByTime(29_000);
    expect(seen).toEqual([]);
    vi.advanceTimersByTime(2_000);
    expect(seen).toHaveLength(1);
    expect(seen[0]).toMatch(/^2026-10-26@/);

    vi.advanceTimersByTime(24 * 60 * 60 * 1000);
    expect(seen).toHaveLength(2);
    expect(seen[1]).toMatch(/^2026-10-27@/);
    stop();
  });

  it("changes when the offset does, while the page stays open", () => {
    // Far from midnight, so the midnight timer cannot be what notices: a
    // daylight-saving change happens in the small hours.
    vi.setSystemTime(new Date(2026, 9, 25, 3, 0, 0));
    const seen: string[] = [];
    const stop = watchLocalDay((key) => seen.push(key));
    const offset = vi
      .spyOn(Date.prototype, "getTimezoneOffset")
      .mockReturnValue(-600);

    vi.advanceTimersByTime(OFFSET_CHECK_MS);
    expect(seen).toEqual(["2026-10-25@600"]);
    // Asked again, nothing moved: nothing said.
    vi.advanceTimersByTime(OFFSET_CHECK_MS);
    expect(seen).toHaveLength(1);
    offset.mockRestore();
    stop();
  });

  it("is checked again when the page comes back into view", () => {
    const seen: string[] = [];
    const stop = watchLocalDay((key) => seen.push(key));
    // Asleep across midnight: the clock moved, no timer ran.
    vi.setSystemTime(new Date(2026, 9, 26, 8, 0, 0));
    document.dispatchEvent(new Event("visibilitychange"));
    expect(seen).toHaveLength(1);
    expect(seen[0]).toMatch(/^2026-10-26@/);
    stop();
  });

  it("stops watching when told to", () => {
    const seen: string[] = [];
    const stop = watchLocalDay((key) => seen.push(key));
    stop();
    vi.advanceTimersByTime(2 * 24 * 60 * 60 * 1000);
    document.dispatchEvent(new Event("visibilitychange"));
    expect(seen).toEqual([]);
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe("the questions that depend on the day", () => {
  const today = "2026-10-25@60";
  const tomorrow = "2026-10-26@60";
  const elsewhere = "2026-10-25@600";

  it("are asked again on another day, or in another offset", () => {
    for (const day of [tomorrow, elsewhere]) {
      expect(mixesQuery("library-1", day).queryKey).not.toEqual(
        mixesQuery("library-1", today).queryKey,
      );
      expect(mixQuery("most-played", "library-1", day).queryKey).not.toEqual(
        mixQuery("most-played", "library-1", today).queryKey,
      );
      expect(statsQuery("30d", day).queryKey).not.toEqual(
        statsQuery("30d", today).queryKey,
      );
    }
    expect(mixesQuery("library-1", today).queryKey).toEqual(
      mixesQuery("library-1", today).queryKey,
    );
  });
});
