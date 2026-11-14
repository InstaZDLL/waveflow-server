import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Song } from "../../../api";
import {
  PlayerContext,
  PlayerProgressContext,
  type PlayerState,
} from "../../../player/context";
import { I18nProvider } from "../../../state/i18n";
import { ClipPlayer, resumeAt, useClipOpen } from "./clip";

const clipUrl = vi.hoisted(() => vi.fn());
vi.mock("../../../api", async (actual) => ({
  ...(await actual<typeof import("../../../api")>()),
  clipUrl,
}));

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}

/**
 * Where the track takes over once its clip closes (RFC-029): at the clip's
 * position, as the desktop does.
 */
describe("resumeAt", () => {
  it("takes over at the clip's position", () => {
    expect(resumeAt(42.5, 200)).toBe(42.5);
    expect(resumeAt(0, 200)).toBe(0);
  });

  it("starts the track over when the clip ran past its end", () => {
    // A video often carries an intro or an outro the album version has not.
    expect(resumeAt(200, 200)).toBe(0);
    expect(resumeAt(245, 200)).toBe(0);
  });

  it("trusts the clip when the track's length is not known yet", () => {
    expect(resumeAt(30, 0)).toBe(30);
  });

  it("leaves the track where it was when the clip never played", () => {
    expect(resumeAt(null, 200)).toBeNull();
    expect(resumeAt(Number.NaN, 200)).toBeNull();
  });

  it("never seeks before the start", () => {
    expect(resumeAt(-3, 200)).toBe(0);
  });
});

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  localStorage.clear();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  clipUrl.mockReset();
});

/** A player that only records what the clip asks of it. */
function fakePlayer(playing: boolean, asidePlaying = false) {
  const player = {
    playing,
    asidePlaying,
    toggle: vi.fn(() => {
      player.playing = !player.playing;
    }),
    toggleAside: vi.fn(() => {
      player.asidePlaying = !player.asidePlaying;
    }),
    seek: vi.fn(),
  };
  return player;
}

const song = { id: "track", title: "Song" } as Song;

/** Renders the clip under `player`; called again, it publishes a new player. */
function renderClip(
  player: ReturnType<typeof fakePlayer>,
  onClose: () => void,
) {
  root.render(
    <I18nProvider>
      <PlayerContext.Provider value={player as unknown as PlayerState}>
        <PlayerProgressContext.Provider value={{ position: 5, duration: 200 }}>
          <ClipPlayer song={song} onClose={onClose} />
        </PlayerProgressContext.Provider>
      </PlayerContext.Provider>
    </I18nProvider>,
  );
}

async function openClip(player: ReturnType<typeof fakePlayer>) {
  const onClose = vi.fn();
  await act(async () => renderClip(player, onClose));
  return onClose;
}

const videoElement = () => container.querySelector("video") as HTMLVideoElement;

/** The element reports a position, as it does while the clip plays. */
function playTo(seconds: number) {
  Object.defineProperty(videoElement(), "currentTime", {
    configurable: true,
    value: seconds,
  });
  act(() => {
    videoElement().dispatchEvent(new Event("timeupdate"));
  });
}

function clickBack() {
  act(() => {
    [...container.querySelectorAll("button")]
      .find((button) => button.textContent === "Back to the track")
      ?.click();
  });
}

describe("ClipPlayer", () => {
  it("hands the track the clip's place even after a refused renewal", async () => {
    clipUrl
      .mockResolvedValueOnce({ url: "/api/v2/clip-stream/one", expiresAt: 1 })
      .mockResolvedValueOnce(null);
    const player = fakePlayer(false);
    const onClose = await openClip(player);
    playTo(60);

    // The ticket ran out during a long pause, and the clip is gone since:
    // the renewal is refused and the element goes away with it.
    await act(async () => {
      videoElement().dispatchEvent(new Event("error"));
    });
    expect(container.querySelector("video")).toBeNull();
    expect(container.textContent).toContain(
      "This clip is no longer on the server.",
    );

    clickBack();
    expect(player.seek).toHaveBeenCalledWith(60);
    expect(onClose).toHaveBeenCalled();
  });

  it("pauses the track whenever the clip starts, one sound at a time", async () => {
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    const player = fakePlayer(true);
    await openClip(player);
    expect(player.toggle).toHaveBeenCalledTimes(1);
    expect(player.playing).toBe(false);

    // The bar takes over, then the clip's own controls start it again.
    player.playing = true;
    act(() => {
      videoElement().dispatchEvent(new Event("play"));
    });
    expect(player.toggle).toHaveBeenCalledTimes(2);
    expect(player.playing).toBe(false);
  });

  it("pauses a station or an episode playing beside the queue", async () => {
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    // The queue is paused while an aside plays, so its state says nothing.
    const player = fakePlayer(false, true);
    await openClip(player);
    act(() => {
      videoElement().dispatchEvent(new Event("play"));
    });
    expect(player.toggleAside).toHaveBeenCalledTimes(1);
    expect(player.asidePlaying).toBe(false);
    expect(player.toggle).not.toHaveBeenCalled();
  });

  it("pauses the clip when a station or an episode starts beside it", async () => {
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    const pause = vi
      .spyOn(HTMLMediaElement.prototype, "pause")
      .mockImplementation(() => {});
    const player = fakePlayer(false, false);
    const onClose = await openClip(player);
    expect(pause).not.toHaveBeenCalled();

    // The provider publishes a new value: a station started from its page.
    act(() => renderClip({ ...player, asidePlaying: true }, onClose));
    expect(pause).toHaveBeenCalledTimes(1);
    pause.mockRestore();
  });
});

function Probe({ trackId }: { trackId: string }) {
  const clip = useClipOpen(trackId);
  return (
    <button type="button" onClick={clip.show}>
      {clip.open ? "open" : "closed"}
    </button>
  );
}

describe("useClipOpen", () => {
  it("closes when the track changes, and stays closed when it comes back", () => {
    act(() => root.render(<Probe trackId="a" />));
    act(() => container.querySelector("button")?.click());
    expect(container.textContent).toBe("open");

    act(() => root.render(<Probe trackId="b" />));
    expect(container.textContent).toBe("closed");
    act(() => root.render(<Probe trackId="a" />));
    expect(container.textContent).toBe("closed");
  });
});
