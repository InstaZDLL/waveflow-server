import { describe, expect, it } from "vitest";

import { resumeAt } from "./clip";

/**
 * Where the track takes over once its clip closes (RFC-029): at the clip's
 * position, as the desktop does.
 */
describe("resumeAt", () => {
  it("takes over at the clip's position", () => {
    expect(resumeAt(42.5, 200)).toBe(42.5);
    expect(resumeAt(0, 200)).toBe(0);
  });

  it("starts the track over when the clip ran past its end", () => {
    // A video often carries an intro or an outro the album version has not.
    expect(resumeAt(200, 200)).toBe(0);
    expect(resumeAt(245, 200)).toBe(0);
  });

  it("trusts the clip when the track's length is not known yet", () => {
    expect(resumeAt(30, 0)).toBe(30);
  });

  it("leaves the track where it was when the clip never played", () => {
    expect(resumeAt(null, 200)).toBeNull();
    expect(resumeAt(Number.NaN, 200)).toBeNull();
  });

  it("never seeks before the start", () => {
    expect(resumeAt(-3, 200)).toBe(0);
  });
});
