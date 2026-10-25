import { describe, expect, it } from "vitest";

import {
  clampPreamp,
  DEFAULT_REPLAY_GAIN,
  parseReplayGainPreference,
  type ReplayGainPreference,
  replayGainFactor,
  routableThroughWebAudio,
} from "./replay-gain";

const decibels = (factor: number) => 20 * Math.log10(factor);

const tagged = {
  replay_gain_track_gain: -7.5,
  replay_gain_track_peak: 0.5,
  replay_gain_album_gain: -6,
  replay_gain_album_peak: 0.25,
};

const mode = (
  value: ReplayGainPreference["mode"],
  preampDb = 0,
): ReplayGainPreference => ({ mode: value, preampDb });

describe("the gain a track is played at", () => {
  it("is unity while ReplayGain is off, whatever the tags say", () => {
    expect(replayGainFactor(tagged, mode("off", 6))).toBe(1);
  });

  it("follows the track gain in track mode and the album gain in album mode", () => {
    expect(decibels(replayGainFactor(tagged, mode("track")))).toBeCloseTo(-7.5);
    expect(decibels(replayGainFactor(tagged, mode("album")))).toBeCloseTo(-6);
  });

  it("falls back to the other pair when the chosen gain is missing", () => {
    const trackOnly = { ...tagged, replay_gain_album_gain: null };
    expect(decibels(replayGainFactor(trackOnly, mode("album")))).toBeCloseTo(
      -7.5,
    );
    const albumOnly = { ...tagged, replay_gain_track_gain: undefined };
    expect(decibels(replayGainFactor(albumOnly, mode("track")))).toBeCloseTo(
      -6,
    );
  });

  it("plays an untagged track at unity, and does not preamp it", () => {
    expect(replayGainFactor({}, mode("album", 6))).toBe(1);
    expect(
      replayGainFactor(
        { replay_gain_track_gain: Number.NaN, replay_gain_track_peak: 0.5 },
        mode("track", 6),
      ),
    ).toBe(1);
  });

  it("adds the preamp to a tagged track", () => {
    expect(decibels(replayGainFactor(tagged, mode("track", 3)))).toBeCloseTo(
      -4.5,
    );
  });

  it("never raises a track past the point its peak would clip", () => {
    // +6 dB on a peak of 0.8 would reach 1.6; the cap holds it at 1 / 0.8.
    const quiet = { replay_gain_track_gain: 6, replay_gain_track_peak: 0.8 };
    expect(replayGainFactor(quiet, mode("track"))).toBeCloseTo(1.25);
    // The peak checked is the one paired with the gain used: the album peak
    // of 0.25 allows up to 4x, so −6 dB plus a 12 dB preamp passes uncapped.
    expect(decibels(replayGainFactor(tagged, mode("album", 12)))).toBeCloseTo(
      6,
    );
  });

  it("ignores a peak that is not a measurement", () => {
    const broken = { replay_gain_track_gain: 6, replay_gain_track_peak: 0 };
    expect(decibels(replayGainFactor(broken, mode("track")))).toBeCloseTo(6);
  });

  it("bounds an absurd tag rather than following it", () => {
    const loud = { replay_gain_track_gain: 80 };
    expect(decibels(replayGainFactor(loud, mode("track")))).toBeCloseTo(18);
    const silent = { replay_gain_track_gain: -500 };
    expect(decibels(replayGainFactor(silent, mode("track")))).toBeCloseTo(-60);
  });
});

describe("the stored preference", () => {
  it("is the default when nothing, or nothing readable, was stored", () => {
    expect(parseReplayGainPreference(null)).toEqual(DEFAULT_REPLAY_GAIN);
    expect(parseReplayGainPreference("not json")).toEqual(DEFAULT_REPLAY_GAIN);
    expect(parseReplayGainPreference("42")).toEqual(DEFAULT_REPLAY_GAIN);
    expect(DEFAULT_REPLAY_GAIN.mode).toBe("off");
  });

  it("reads back what was written", () => {
    expect(
      parseReplayGainPreference(
        JSON.stringify({ mode: "album", preampDb: -3 }),
      ),
    ).toEqual({ mode: "album", preampDb: -3 });
  });

  it("keeps a valid mode beside a preamp out of range, clamped", () => {
    expect(
      parseReplayGainPreference(
        JSON.stringify({ mode: "track", preampDb: 20 }),
      ),
    ).toEqual({ mode: "track", preampDb: 12 });
    expect(
      parseReplayGainPreference(JSON.stringify({ mode: "loud", preampDb: 2 })),
    ).toEqual({ mode: "off", preampDb: 2 });
  });

  it("rounds a preamp to whole decibels", () => {
    expect(clampPreamp(2.6)).toBe(3);
    expect(clampPreamp(-13)).toBe(-12);
    expect(clampPreamp(Number.POSITIVE_INFINITY)).toBe(0);
  });
});

describe("which media may be routed through Web Audio", () => {
  const page = "https://music.example.com/albums/1";

  it("accepts a stream ticket, which is a relative URL", () => {
    expect(routableThroughWebAudio("/api/v2/stream/abc", page)).toBe(true);
  });

  it("refuses media on another origin, which would play as silence", () => {
    expect(routableThroughWebAudio("https://cdn.example.net/a.mp3", page)).toBe(
      false,
    );
    expect(routableThroughWebAudio("http://music.example.com/a", page)).toBe(
      false,
    );
  });
});
