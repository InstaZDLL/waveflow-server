import { describe, expect, it } from "vitest";

import type { Song } from "./api";
import {
  QUALITY_STEPS,
  type QualityBand,
  qualityBand,
  qualitySteps,
} from "./file-quality";

/**
 * The band is client logic, so it is tested here. **What the server sends is
 * not**, and that distinction is the whole reason this file is short: a
 * fixture built from `Song` agrees with `Song` by construction, so no test on
 * this side can establish that `bit_depth` arrives at all.
 * `the_technical_fields_reach_the_wire_and_match_the_file` does that, against
 * the bytes of a real file.
 */
function song(fields: Partial<Song>): Song {
  return {
    id: "song-1",
    library_id: "library-1",
    album_id: null,
    title: "Army of Me",
    album: null,
    artist: null,
    artist_id: null,
    artwork_hash: null,
    duration_ms: 234_000,
    track: null,
    disc: null,
    starred_at: null,
    user_rating: null,
    codec: null,
    suffix: "flac",
    size: 1024,
    bitrate: null,
    sample_rate: null,
    channels: null,
    bit_depth: null,
    ...fields,
  };
}

describe("the quality band", () => {
  it("reads lossless off the bit depth, not off a codec name", () => {
    // The load-bearing claim. `bit_depth` is present only where the file's
    // reader found one, which no lossy format offers — so the client needs no
    // list of codec names, and a format nobody here has heard of is graded by
    // the server rather than misgraded by this function.
    expect(qualityBand(song({ bit_depth: 16, sample_rate: 44_100 }))).toBe(
      "lossless",
    );
    // And a quiet recording that compressed hard is still lossless. A bitrate
    // threshold would have called this one "high" at best, which is the
    // mistake the bit depth exists here to prevent.
    expect(
      qualityBand(song({ bit_depth: 16, sample_rate: 44_100, bitrate: 240 })),
    ).toBe("lossless");
  });

  it("separates hi-res by depth or by rate, either one", () => {
    expect(qualityBand(song({ bit_depth: 24, sample_rate: 44_100 }))).toBe(
      "hiRes",
    );
    expect(qualityBand(song({ bit_depth: 16, sample_rate: 96_000 }))).toBe(
      "hiRes",
    );
    // 48 kHz is the boundary and is not above it: it is an ordinary
    // production rate, not a high-resolution one.
    expect(qualityBand(song({ bit_depth: 16, sample_rate: 48_000 }))).toBe(
      "lossless",
    );
  });

  it("grades a lossy file by the only axis it offers", () => {
    expect(qualityBand(song({ bitrate: 320 }))).toBe("high");
    expect(qualityBand(song({ bitrate: 256 }))).toBe("high");
    expect(qualityBand(song({ bitrate: 255 }))).toBe("standard");
    expect(qualityBand(song({ bitrate: 128 }))).toBe("standard");
    expect(qualityBand(song({ bitrate: 127 }))).toBe("low");
  });

  it("treats a key the response never sent as absent, not as a reading", () => {
    // `api.ts` declares `number | null`, and a declaration is a claim about
    // the server rather than a guarantee from it. A response missing the key
    // gives `undefined`, which `!== null` waves through — the first version
    // of this function then called such a file lossless on the strength of a
    // field it had never received. The cast is the point: this is the shape a
    // type cannot forbid.
    const missing = { ...song({}) } as Record<string, unknown>;
    for (const key of ["bit_depth", "sample_rate", "bitrate"]) {
      delete missing[key];
    }
    expect(qualityBand(missing as unknown as Song)).toBe("unknown");
  });

  it("says unknown rather than low when there is nothing to go on", () => {
    // A file the reader could not measure is not a bad file. Grading it `low`
    // would be stating a measurement that was never taken.
    expect(qualityBand(song({}))).toBe("unknown");
    expect(qualitySteps("unknown")).toBe(0);
  });

  it("lights more steps for a better band, and never more than the scale", () => {
    const rising: QualityBand[] = [
      "unknown",
      "low",
      "standard",
      "high",
      "lossless",
      "hiRes",
    ];
    const steps = rising.map(qualitySteps);
    expect(steps).toEqual([...steps].sort((a, b) => a - b));
    expect(new Set(steps).size).toBe(rising.length);
    expect(Math.max(...steps)).toBe(QUALITY_STEPS);
  });
});
