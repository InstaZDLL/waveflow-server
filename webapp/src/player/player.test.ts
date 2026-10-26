import { afterEach, describe, expect, it, vi } from "vitest";

import {
  setAsideMediaSessionHandlers,
  setDirectionalMediaSessionHandlers,
} from "./player";

function recordingSession() {
  const handlers = new Map<
    MediaSessionAction,
    MediaSessionActionHandler | null
  >();
  const mediaSession = {
    setActionHandler: (
      action: MediaSessionAction,
      handler: MediaSessionActionHandler | null,
    ) => handlers.set(action, handler),
  };
  return { handlers, mediaSession };
}

describe("Media Session actions for a station or an episode", () => {
  it("retries a failed source on the system's Play, and leaves its Pause alone", () => {
    const { handlers, mediaSession } = recordingSession();
    // What a failed station looks like: `paused` still false, an error set.
    const element = {
      paused: false,
      error: { code: 4 } as MediaError | null,
    };
    const toggle = vi.fn();
    setAsideMediaSessionHandlers(mediaSession, element, toggle);

    // The system shows Play, because nothing sounds. Pressing Pause must not
    // start the station again, and pressing Play must try it.
    handlers.get("pause")?.({ action: "pause" });
    expect(toggle).not.toHaveBeenCalled();
    handlers.get("play")?.({ action: "play" });
    expect(toggle).toHaveBeenCalledOnce();
  });

  it("pauses a source that sounds, and ignores Play on it", () => {
    const { handlers, mediaSession } = recordingSession();
    const element = { paused: false, error: null };
    const toggle = vi.fn();
    setAsideMediaSessionHandlers(mediaSession, element, toggle);

    handlers.get("play")?.({ action: "play" });
    expect(toggle).not.toHaveBeenCalled();
    handlers.get("pause")?.({ action: "pause" });
    expect(toggle).toHaveBeenCalledOnce();
  });
});

afterEach(() => {
  Reflect.deleteProperty(navigator, "mediaSession");
});

describe("Media Session playback actions", () => {
  it("keeps play and pause directional", () => {
    const handlers = new Map<
      MediaSessionAction,
      MediaSessionActionHandler | null
    >();
    const mediaSession = {
      setActionHandler: vi.fn(
        (
          action: MediaSessionAction,
          handler: MediaSessionActionHandler | null,
        ) => handlers.set(action, handler),
      ),
    } as unknown as MediaSession;
    Object.defineProperty(navigator, "mediaSession", {
      configurable: true,
      value: mediaSession,
    });

    let paused = true;
    const element = {
      get paused() {
        return paused;
      },
    };
    const play = vi.fn(() => {
      paused = false;
    });
    const pause = vi.fn(() => {
      paused = true;
    });

    setDirectionalMediaSessionHandlers(
      navigator.mediaSession,
      element,
      play,
      pause,
    );
    handlers.get("play")?.({ action: "play" });
    handlers.get("play")?.({ action: "play" });
    expect(play).toHaveBeenCalledOnce();
    expect(pause).not.toHaveBeenCalled();

    handlers.get("pause")?.({ action: "pause" });
    handlers.get("pause")?.({ action: "pause" });
    expect(pause).toHaveBeenCalledOnce();
  });
});
