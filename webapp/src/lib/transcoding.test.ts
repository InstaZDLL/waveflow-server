import { describe, expect, it } from "vitest";

import type { TranscodeStatus } from "../api";
import {
  formatLabel,
  offeredFormats,
  readBitrate,
  transcodeLoad,
} from "./transcoding";

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

describe("formatLabel", () => {
  it("writes the formats the way a reader does", () => {
    expect(formatLabel("opus")).toBe("Opus");
    expect(formatLabel("mp3")).toBe("MP3");
    expect(formatLabel("flac")).toBe("FLAC");
  });
});

describe("offeredFormats", () => {
  it("offers what the server produces", () => {
    expect(offeredFormats(["mp3", "opus"], "opus")).toEqual(["mp3", "opus"]);
    expect(offeredFormats(["mp3", "opus"], "raw")).toEqual(["mp3", "opus"]);
  });

  it("keeps a format the server no longer lists, so the select can show it", () => {
    expect(offeredFormats(["mp3"], "opus")).toEqual(["mp3", "opus"]);
  });
});

describe("readBitrate", () => {
  it("reads an empty field as the format's default", () => {
    expect(readBitrate("")).toBeNull();
    expect(readBitrate("  ")).toBeNull();
  });

  it("holds the bounds the server holds", () => {
    expect(readBitrate("32")).toBe(32);
    expect(readBitrate(" 512 ")).toBe(512);
    expect(readBitrate("31")).toBe("invalid");
    expect(readBitrate("513")).toBe("invalid");
    expect(readBitrate("128.5")).toBe("invalid");
    expect(readBitrate("loud")).toBe("invalid");
  });
});
