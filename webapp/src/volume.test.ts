import { afterEach, describe, expect, it } from "vitest";

import { readStoredVolume } from "./player";

afterEach(() => localStorage.clear());

/**
 * The level a fresh browser starts at. This read `Number(getItem(...))`
 * directly, and `Number(null)` is 0 — finite, and inside the accepted range —
 * so every browser that had never stored a level started the player silent,
 * with the mute button reporting that nothing was muted.
 */
describe("readStoredVolume", () => {
  it("is full volume when nothing has ever been stored", () => {
    expect(readStoredVolume()).toBe(1);
  });

  it("is full volume for a stored value that says nothing", () => {
    // Both convert to 0 without a stored level ever having been chosen.
    localStorage.setItem("waveflow.volume", "");
    expect(readStoredVolume()).toBe(1);
    localStorage.setItem("waveflow.volume", "   ");
    expect(readStoredVolume()).toBe(1);
  });

  it("keeps a level that was really stored, silence included", () => {
    localStorage.setItem("waveflow.volume", "0");
    expect(readStoredVolume()).toBe(0);
    localStorage.setItem("waveflow.volume", "0.35");
    expect(readStoredVolume()).toBe(0.35);
    localStorage.setItem("waveflow.volume", "1");
    expect(readStoredVolume()).toBe(1);
  });

  it("refuses what is not a level", () => {
    for (const stored of ["loud", "2", "-0.5", "NaN", "Infinity"]) {
      localStorage.setItem("waveflow.volume", stored);
      expect(readStoredVolume()).toBe(1);
    }
  });
});
