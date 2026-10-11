import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { dismissSplash } from "./splash";

/**
 * The floor is tested here rather than against a browser on purpose.
 *
 * An end-to-end assertion would have to race a real load: it could only read
 * the instant the screen left, which is `max(floor, mount) + fade`. A CI
 * runner that takes a second to parse 276 kB of React would fail a bound that
 * is about the floor and not about the machine — the shape of flake this
 * repository spent a morning on. Fake timers measure the floor itself.
 */
describe("the first-load screen", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    document.body.innerHTML = `<div id="splash"><img src="/logo.svg" alt=""></div>`;
  });

  afterEach(() => {
    vi.useRealTimers();
    document.body.innerHTML = "";
  });

  const splash = () => document.getElementById("splash");

  it("holds the mark to the floor when the application is ready sooner", () => {
    // Ready at 40ms: a mark drawn and yanked inside one blink reads as a
    // glitch, which is what the floor exists to prevent.
    vi.setSystemTime(40);
    vi.advanceTimersByTime(40);
    dismissSplash();

    vi.advanceTimersByTime(250);
    expect(splash()).not.toBeNull();
    expect(splash()?.dataset.leaving).not.toBe("true");

    // Floor reached: the fade starts, and the element is still in the document
    // for as long as it is fading.
    vi.advanceTimersByTime(20);
    expect(splash()?.dataset.leaving).toBe("true");
    expect(splash()).not.toBeNull();

    vi.advanceTimersByTime(180);
    // Removed, not merely transparent: an element over the whole viewport
    // still takes a click at zero opacity.
    expect(splash()).toBeNull();
  });

  it("adds no wait of its own when the application is ready later", () => {
    // Ready at 900ms, past the floor. The screen must leave at once: a floor
    // that kept adding time would make every slow load slower.
    vi.advanceTimersByTime(900);
    dismissSplash();

    vi.advanceTimersByTime(0);
    expect(splash()?.dataset.leaving).toBe("true");
    vi.advanceTimersByTime(180);
    expect(splash()).toBeNull();
  });

  /**
   * The bound the issue is emphatic about: **not** one to two seconds.
   *
   * A longer floor would make the application permanently slower for everyone
   * to hide a load that takes 79ms, and it is the kind of change nobody could
   * take back later without it being noticed. Read off the module rather than
   * written down twice, so raising the constant fails here.
   */
  it("leaves well inside a second on a load that is already done", () => {
    vi.advanceTimersByTime(1);
    dismissSplash();
    vi.advanceTimersByTime(600);
    expect(splash()).toBeNull();
  });

  it("does nothing on a second call", () => {
    vi.advanceTimersByTime(400);
    dismissSplash();
    // StrictMode runs an effect twice in development. A second schedule would
    // remove an element the first one had already taken, or worse, take a
    // replacement.
    dismissSplash();
    vi.advanceTimersByTime(180);
    expect(splash()).toBeNull();
  });

  it("is harmless where the screen was never rendered", () => {
    document.body.innerHTML = "";
    expect(() => dismissSplash()).not.toThrow();
  });
});
