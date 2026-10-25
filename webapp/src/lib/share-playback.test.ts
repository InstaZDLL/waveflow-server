import { describe, expect, it } from "vitest";

import { shareSource } from "./share-playback";

const stream = "/share/wfs_token/tracks/track-id/stream?visit=pass";

describe("shareSource", () => {
  it("plays the original when the browser can", () => {
    expect(shareSource(stream, "FLAC", () => "maybe")).toBe(stream);
  });

  it("asks for the fixed MP3 when the browser cannot play the original", () => {
    expect(shareSource(stream, "flac", () => "")).toBe(`${stream}&format=mp3`);
  });

  it("asks for the MP3 for a format it has no media type for", () => {
    let asked = false;
    const source = shareSource(stream, "dsf", () => {
      asked = true;
      return "probably";
    });
    expect(source).toBe(`${stream}&format=mp3`);
    expect(asked).toBe(false);
  });

  it("starts the query when the address has none", () => {
    expect(shareSource("/stream", "ape", () => "")).toBe("/stream?format=mp3");
  });
});
