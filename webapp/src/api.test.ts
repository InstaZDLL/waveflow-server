import { describe, expect, it } from "vitest";

import { isAllowedRedirect, safeInternalPath } from "./api";

/**
 * These two guards are the client half of the OAuth redirect policy. Both were
 * added after review found the consent screen and the post-login hop navigating
 * to attacker-supplied targets, so they are covered here rather than left to
 * inspection.
 */
describe("isAllowedRedirect", () => {
  it("accepts what a native client can prove it controls", () => {
    expect(isAllowedRedirect("http://127.0.0.1:49152/cb")).toBe(true);
    expect(isAllowedRedirect("http://localhost:1234/cb")).toBe(true);
    expect(isAllowedRedirect("http://[::1]:5000/cb")).toBe(true);
    expect(isAllowedRedirect("https://desktop.example.com/cb")).toBe(true);
    expect(isAllowedRedirect("com.waveflow.desktop://auth")).toBe(true);
  });

  it("refuses targets that would carry the code elsewhere", () => {
    // Clear text to a remote host leaks the code off the machine.
    expect(isAllowedRedirect("http://evil.example.com/cb")).toBe(false);
    // A bare scheme is claimable by any other application.
    expect(isAllowedRedirect("waveflow://auth")).toBe(false);
    expect(isAllowedRedirect("javascript:alert(1)")).toBe(false);
    expect(isAllowedRedirect("not a url")).toBe(false);
    expect(isAllowedRedirect("")).toBe(false);
  });

  it("refuses a fragment, which the server also rejects", () => {
    expect(isAllowedRedirect("https://desktop.example.com/cb#frag")).toBe(
      false,
    );
  });
});

describe("safeInternalPath", () => {
  it("keeps a same-document path", () => {
    expect(safeInternalPath("/albums")).toBe("/albums");
    expect(safeInternalPath("/authorize?client_id=x&state=y")).toBe(
      "/authorize?client_id=x&state=y",
    );
  });

  it("refuses anything that leaves the origin", () => {
    // Protocol-relative: location.assign would follow it to another host.
    expect(safeInternalPath("//evil.example.com")).toBeNull();
    // Browsers normalise backslashes, so this escapes just like "//".
    expect(safeInternalPath("/\\evil.example.com")).toBeNull();
    expect(safeInternalPath("https://evil.example.com")).toBeNull();
    expect(safeInternalPath("javascript:alert(1)")).toBeNull();
    expect(safeInternalPath("albums")).toBeNull();
    expect(safeInternalPath(null)).toBeNull();
    expect(safeInternalPath("")).toBeNull();
  });
});
