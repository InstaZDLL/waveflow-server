import { describe, expect, it } from "vitest";
import type { UpdateStatus } from "../api/server-update";
import { updateNotice } from "./update-notice";

const base: UpdateStatus = {
  policy: "ask",
  decision: true,
  enabled: true,
  current: "2.1.2",
  channel: "stable",
  latest: {
    version: "2.2.0",
    tag: "v2.2.0",
    published_at: "2026-11-10T00:00:00Z",
    page: "https://github.com/InstaZDLL/waveflow-server/releases/tag/v2.2.0",
    notes: null,
  },
  update_available: true,
  checked_at: 1,
  failure: null,
};

describe("updateNotice", () => {
  it("says nothing before the status is known", () => {
    expect(updateNotice(undefined, null)).toBeNull();
  });

  it("asks while nobody has answered, whatever else is known", () => {
    expect(updateNotice({ ...base, decision: null }, null)).toEqual({
      kind: "ask",
    });
  });

  it("never asks when the environment decided", () => {
    for (const policy of ["on", "off"] as const) {
      const notice = updateNotice(
        { ...base, policy, decision: null, update_available: false },
        null,
      );
      expect(notice).toBeNull();
    }
  });

  it("announces a newer version with its page", () => {
    expect(updateNotice(base, null)).toEqual({
      kind: "update",
      version: "2.2.0",
      page: base.latest?.page,
    });
  });

  it("stays quiet about a version this browser dismissed, not the next", () => {
    expect(updateNotice(base, "2.2.0")).toBeNull();
    expect(updateNotice(base, "2.1.9")).not.toBeNull();
  });

  it("says nothing when the version is not newer, or nothing was read", () => {
    expect(updateNotice({ ...base, update_available: false }, null)).toBeNull();
    expect(updateNotice({ ...base, latest: null }, null)).toBeNull();
  });
});
