import { describe, expect, it } from "vitest";

import type { TranscodeStatus } from "../api";
import { transcodeLoad } from "./transcoding";

const status = (overrides: Partial<TranscodeStatus> = {}): TranscodeStatus => ({
  available: true,
  active: 0,
  global_limit: 4,
  per_user_limit: 1,
  formats: ["mp3", "opus", "aac"],
  ...overrides,
});

describe("transcodeLoad", () => {
  it("says a server that cannot transcode is unavailable, whatever it counts", () => {
    expect(transcodeLoad(status({ available: false }))).toBe("unavailable");
    expect(transcodeLoad(status({ available: false, active: 4 }))).toBe(
      "unavailable",
    );
  });

  it("is full on reaching the limit, not only past it", () => {
    expect(transcodeLoad(status({ active: 3 }))).toBe("open");
    // The server refuses the request that would make a fifth, so four of four
    // is already a refusal for the next listener.
    expect(transcodeLoad(status({ active: 4 }))).toBe("full");
    expect(transcodeLoad(status({ active: 5 }))).toBe("full");
  });

  it("leaves an idle server open", () => {
    expect(transcodeLoad(status())).toBe("open");
  });
});
