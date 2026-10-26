import { describe, expect, it, vi } from "vitest";

import { onMediaChange } from "./media-query";

describe("onMediaChange", () => {
  it("listens through addEventListener where the browser has it", () => {
    const query = {
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
    } as unknown as MediaQueryList;
    const listener = () => {};
    const stop = onMediaChange(query, listener);
    expect(query.addEventListener).toHaveBeenCalledWith("change", listener);
    expect(query.addListener).not.toHaveBeenCalled();
    stop();
    expect(query.removeEventListener).toHaveBeenCalledWith("change", listener);
  });

  // Safari before 14: only the deprecated pair exists, and reaching for the
  // missing one would throw.
  it("falls back to addListener on an older MediaQueryList", () => {
    const query = {
      addListener: vi.fn(),
      removeListener: vi.fn(),
    } as unknown as MediaQueryList;
    const listener = () => {};
    const stop = onMediaChange(query, listener);
    expect(query.addListener).toHaveBeenCalledWith(listener);
    stop();
    expect(query.removeListener).toHaveBeenCalledWith(listener);
  });

  it("does nothing, and throws nothing, where neither exists", () => {
    const query = {} as MediaQueryList;
    expect(() => onMediaChange(query, () => {})()).not.toThrow();
  });
});
