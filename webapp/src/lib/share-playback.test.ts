import { describe, expect, it } from "vitest";

import { sharePlayer, shareSource } from "./share-playback";

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

describe("sharePlayer", () => {
  const tracks = [
    { streamUrl: "/share/wfs_token/tracks/one/stream?visit=a", suffix: "flac" },
    { streamUrl: "/share/wfs_token/tracks/two/stream?visit=b", suffix: "dsf" },
  ];
  const canPlay = () => "maybe";

  // Chrome on Android, 2026-10-28: the player's own play button sat at
  // 0:00 / 0:00 with "Unable to play media" until a row was touched, because
  // the element had no source before one was.
  it("gives the player's own button what the first row plays", () => {
    const idle = sharePlayer(tracks, null, canPlay);
    const firstRow = sharePlayer(tracks, 0, canPlay);
    expect(idle.src).toBeDefined();
    expect(idle.src).toBe(firstRow.src);
    expect(idle.position).toBe(0);
  });

  it("plays and fetches nothing until something is chosen", () => {
    expect(sharePlayer(tracks, null, canPlay)).toMatchObject({
      autoPlay: false,
      preload: "none",
    });
    expect(sharePlayer(tracks, 1, canPlay)).toMatchObject({
      autoPlay: true,
      preload: "auto",
    });
  });

  it("plays the row chosen, through the same choice of source", () => {
    expect(sharePlayer(tracks, 1, canPlay).src).toBe(
      shareSource(tracks[1].streamUrl, tracks[1].suffix, canPlay),
    );
  });

  it("has nothing to play in an empty share", () => {
    expect(sharePlayer([], null, canPlay).src).toBeUndefined();
  });
});
