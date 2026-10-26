import { describe, expect, it } from "vitest";

import type { PodcastEpisode, RadioStation } from "../api";
import {
  asideArtwork,
  asideMetadata,
  asideTransport,
  bookmarkWrite,
  dueForSave,
  reportsPlaybackFailure,
  resumeSeconds,
  SAVE_EVERY_MS,
  shownDuration,
  sounding,
} from "./aside";

describe("whether the aside sounds", () => {
  it("is not paused and not failed", () => {
    expect(sounding({ paused: false, error: null })).toBe(true);
    expect(sounding({ paused: true, error: null })).toBe(false);
    // A failed source leaves `paused` false: it is still silent.
    expect(sounding({ paused: false, error: { code: 2 } })).toBe(false);
    expect(sounding({ paused: true, error: { code: 2 } })).toBe(false);
  });
});

const station: RadioStation = {
  id: "station-1",
  name: "Radio Nova",
  stream_url: "https://nova.example.org/live",
  homepage_url: null,
  image_url: null,
  artwork_hash: null,
  created_at: 1,
  updated_at: 1,
};

const episode: PodcastEpisode = {
  id: "episode-1",
  channel_id: "channel-1",
  channel_title: "Night Shift",
  artwork_hash: null,
  title: "First night",
  description: null,
  published_at: null,
  duration_ms: 720_000,
  status: "completed",
  error: null,
  size: null,
  suffix: "mp3",
  content_type: "audio/mpeg",
  position_ms: null,
  created_at: 1,
};

describe("the cover an aside shows", () => {
  it("is the station's image, or the episode's channel's cover", () => {
    expect(asideArtwork({ kind: "live", station })).toBeNull();
    expect(
      asideArtwork({
        kind: "live",
        station: { ...station, artwork_hash: "s" },
      }),
    ).toBe("s");
    expect(
      asideArtwork({
        kind: "episode",
        episode: { ...episode, artwork_hash: "c" },
      }),
    ).toBe("c");
  });

  it("is never the address an image came from", () => {
    // A station whose image could not be fetched keeps its address and shows
    // the symbol: the browser does not load another host's image.
    expect(
      asideArtwork({
        kind: "live",
        station: { ...station, image_url: "https://logos.example.org/n.png" },
      }),
    ).toBeNull();
  });
});

describe("the transport an aside offers", () => {
  it("has no position and nowhere to seek for a station", () => {
    expect(asideTransport("live")).toEqual({ progress: false, seek: false });
  });

  it("has both for an episode", () => {
    expect(asideTransport("episode")).toEqual({ progress: true, seek: true });
  });
});

describe("what the Media Session shows", () => {
  const labels = { live: "Live radio", podcasts: "Podcasts" };

  it("names a station and says it is live", () => {
    expect(asideMetadata({ kind: "live", station }, labels)).toEqual({
      title: "Radio Nova",
      artist: "Live radio",
      album: "WaveFlow",
    });
  });

  it("names an episode by its title and its channel", () => {
    expect(asideMetadata({ kind: "episode", episode }, labels)).toEqual({
      title: "First night",
      artist: "Night Shift",
      album: "Podcasts",
    });
  });
});

describe("where an episode resumes", () => {
  it("starts at its bookmark", () => {
    expect(resumeSeconds({ position_ms: 30_000, duration_ms: 720_000 })).toBe(
      30,
    );
  });

  it("starts over without one, or with one at the very start", () => {
    expect(resumeSeconds({ position_ms: null, duration_ms: 720_000 })).toBe(0);
    expect(resumeSeconds({ position_ms: 0, duration_ms: 720_000 })).toBe(0);
  });

  it("starts over when it was left in its last seconds", () => {
    expect(resumeSeconds({ position_ms: 716_000, duration_ms: 720_000 })).toBe(
      0,
    );
    // An episode of unknown length resumes wherever it was left.
    expect(resumeSeconds({ position_ms: 716_000, duration_ms: null })).toBe(
      716,
    );
  });
});

describe("an episode's bookmark", () => {
  it("keeps the place, in whole milliseconds", () => {
    expect(bookmarkWrite(12.3456, false)).toEqual({
      kind: "set",
      positionMs: 12_346,
    });
    expect(bookmarkWrite(-1, false)).toEqual({ kind: "set", positionMs: 0 });
  });

  it("is forgotten once the episode was heard to its end", () => {
    expect(bookmarkWrite(720, true)).toEqual({ kind: "delete" });
  });

  it("is written again only once the interval has passed", () => {
    expect(dueForSave(1_000 + SAVE_EVERY_MS - 1, 1_000)).toBe(false);
    expect(dueForSave(1_000 + SAVE_EVERY_MS, 1_000)).toBe(true);
  });
});

describe("the duration the bar shows", () => {
  it("is none for a live stream or for nothing loaded", () => {
    expect(shownDuration(Number.POSITIVE_INFINITY)).toBe(0);
    expect(shownDuration(Number.NaN)).toBe(0);
    expect(shownDuration(0)).toBe(0);
    expect(shownDuration(42.5)).toBe(42.5);
  });
});

describe("a play attempt that failed", () => {
  it("reports only when it is still the one on screen, and not when it was interrupted", () => {
    // The ordinary failure: this attempt is current, and the station is
    // genuinely unreachable.
    expect(
      reportsPlaybackFailure(3, 3, new DOMException("no", "NotSupportedError")),
    ).toBe(true);
    // An element error carries no failure value at all, and still reports.
    expect(reportsPlaybackFailure(3, 3)).toBe(true);

    // A second station was started while this one was loading. Its rejection
    // arrives after the new station is on screen; blaming it would put an
    // error over a station that is playing.
    expect(
      reportsPlaybackFailure(2, 3, new DOMException("aborted", "AbortError")),
    ).toBe(false);
    expect(reportsPlaybackFailure(2, 3)).toBe(false);

    // Stop was pressed while this one was loading. `pause()` rejects the
    // pending promise with `AbortError`, which is the player obeying rather
    // than a station that cannot be reached — and the counter has moved, so
    // either rule alone would catch this one.
    expect(
      reportsPlaybackFailure(3, 3, new DOMException("aborted", "AbortError")),
    ).toBe(false);
  });
});
