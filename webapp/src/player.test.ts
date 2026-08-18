import { afterEach, describe, expect, it, vi } from "vitest";

import { setDirectionalMediaSessionHandlers } from "./player";

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
