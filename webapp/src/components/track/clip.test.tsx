import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Song } from "../../api";
import {
  PlayerContext,
  PlayerProgressContext,
  type PlayerState,
} from "../../player/context";
import { I18nProvider } from "../../state/i18n";
import { ClipPlayer, type ClipSession, resumeAt, useClipOpen } from "./clip";
import { ClipDialog } from "./clip-dialog";

const clipUrl = vi.hoisted(() => vi.fn());
vi.mock("../../api", async (actual) => ({
  ...(await actual<typeof import("../../api")>()),
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
    /** The track in the queue: the one whose clip opens, unless a test says. */
    current: { id: "track" } as { id: string } | null,
    playing,
    asidePlaying,
    /** What plays beside the queue: set while one is loaded, playing or not. */
    aside: (asidePlaying ? { kind: "station" } : null) as object | null,
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

/** Renders `node` under `player`; called again, it publishes a new player. */
function renderUnder(player: ReturnType<typeof fakePlayer>, node: ReactNode) {
  root.render(
    <I18nProvider>
      <PlayerContext.Provider value={player as unknown as PlayerState}>
        <PlayerProgressContext.Provider value={{ position: 5, duration: 200 }}>
          {node}
        </PlayerProgressContext.Provider>
      </PlayerContext.Provider>
    </I18nProvider>,
  );
}

/** Renders the clip under `player`; called again, it publishes a new player. */
function renderClip(
  player: ReturnType<typeof fakePlayer>,
  onClose: () => void,
) {
  renderUnder(player, <ClipPlayer song={song} onClose={onClose} />);
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

  it("does not take the track back over a station started meanwhile", async () => {
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    const player = fakePlayer(true);
    const onClose = await openClip(player);
    expect(player.toggle).toHaveBeenCalledTimes(1);

    // A station started from its page while the clip was open.
    player.aside = { kind: "station" };
    player.asidePlaying = true;
    clickBack();
    expect(player.toggle).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalled();
  });

  it("leaves a station the clip paused in the bar when it closes", async () => {
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    const player = fakePlayer(true);
    await openClip(player);

    // A station started meanwhile, then paused by the clip playing again: it
    // no longer sounds, but it is still the listener's choice in the bar.
    player.aside = { kind: "station" };
    player.asidePlaying = true;
    act(() => {
      videoElement().dispatchEvent(new Event("play"));
    });
    expect(player.asidePlaying).toBe(false);
    clickBack();
    expect(player.toggle).toHaveBeenCalledTimes(1);
  });

  it("takes the track back when nothing else started", async () => {
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    const player = fakePlayer(true);
    await openClip(player);
    clickBack();
    expect(player.toggle).toHaveBeenCalledTimes(2);
    expect(player.playing).toBe(true);
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

  it("leaves another track where it was, and gives it back", async () => {
    // Opened from a track's menu while something else is in the queue: the
    // clip's position is a place in a different recording.
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    const player = fakePlayer(true);
    player.current = { id: "another" };
    await openClip(player);
    expect(player.playing).toBe(false);
    playTo(60);
    clickBack();
    expect(player.seek).not.toHaveBeenCalled();
    expect(player.playing).toBe(true);
  });

  it("does not start a track the listener changed to meanwhile", async () => {
    // The dialog is modal, the media keys are not: the queue moved on, paused,
    // while the clip played, and the track there now is not the one it paused.
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    const player = fakePlayer(true);
    player.current = { id: "another" };
    await openClip(player);
    expect(player.toggle).toHaveBeenCalledTimes(1);

    player.current = { id: "next-in-queue" };
    clickBack();
    expect(player.toggle).toHaveBeenCalledTimes(1);
    expect(player.playing).toBe(false);
  });

  it("leaves the clip's own track where it plays when Next reached it", async () => {
    // A plays, the clip is B's, opened from its menu; the Next key starts B,
    // which pauses the clip. B plays from its own start, not the clip's place.
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    const player = fakePlayer(true);
    player.current = { id: "another" };
    await openClip(player);
    playTo(60);

    player.current = { id: "track" };
    player.playing = true;
    clickBack();
    expect(player.seek).not.toHaveBeenCalled();
  });
});

/**
 * The detail view and the immersive view each show the clip in their own
 * place, so moving between them mounts a new player on the same session.
 */
describe("ClipPlayer carried from one view to the next", () => {
  function session(): ClipSession {
    return { wasPlaying: null, openedOn: null, at: null, playing: true };
  }

  it("goes on where it stood, and still gives the track back", async () => {
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    const carried = session();
    const player = fakePlayer(true);
    const onClose = vi.fn();
    await act(async () =>
      renderUnder(
        player,
        <ClipPlayer
          key="detail"
          song={song}
          onClose={onClose}
          session={carried}
        />,
      ),
    );
    playTo(60);
    // The track was paused by the clip; the next view must not read that.
    expect(player.playing).toBe(false);

    await act(async () =>
      renderUnder(
        player,
        <ClipPlayer
          key="immersive"
          song={song}
          onClose={onClose}
          session={carried}
        />,
      ),
    );
    expect(videoElement().autoplay).toBe(true);
    act(() => {
      videoElement().dispatchEvent(new Event("loadedmetadata"));
    });
    expect(videoElement().currentTime).toBe(60);

    playTo(75);
    clickBack();
    expect(player.seek).toHaveBeenCalledWith(75);
    expect(player.playing).toBe(true);
    expect(onClose).toHaveBeenCalled();
  });

  it("hands on the place it was carried to, before the element has it", async () => {
    // Closed just after a change of view: the new element has no metadata yet
    // and reads 0, while the clip stood at a minute.
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    const carried: ClipSession = {
      wasPlaying: true,
      openedOn: "track",
      at: 60,
      playing: true,
    };
    const player = fakePlayer(false);
    await act(async () =>
      renderUnder(
        player,
        <ClipPlayer song={song} onClose={vi.fn()} session={carried} />,
      ),
    );
    expect(videoElement().currentTime).toBe(0);
    clickBack();
    expect(player.seek).toHaveBeenCalledWith(60);
  });

  it("keeps that place through a renewal that fails before the metadata", async () => {
    clipUrl
      .mockResolvedValueOnce({ url: "/api/v2/clip-stream/one", expiresAt: 1 })
      .mockResolvedValueOnce({ url: "/api/v2/clip-stream/two", expiresAt: 2 });
    const carried: ClipSession = {
      wasPlaying: false,
      openedOn: "track",
      at: 60,
      playing: true,
    };
    const player = fakePlayer(false);
    await act(async () =>
      renderUnder(
        player,
        <ClipPlayer song={song} onClose={vi.fn()} session={carried} />,
      ),
    );
    await act(async () => {
      videoElement().dispatchEvent(new Event("error"));
    });
    act(() => {
      videoElement().dispatchEvent(new Event("loadedmetadata"));
    });
    expect(videoElement().currentTime).toBe(60);
  });

  it("reads the track it opened over from the session", async () => {
    // The player taking over must not take the track current at the change
    // of view for the one the clip paused.
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    const carried: ClipSession = {
      wasPlaying: true,
      openedOn: "track",
      at: 10,
      playing: true,
    };
    const player = fakePlayer(false);
    player.current = { id: "other" };
    await act(async () =>
      renderUnder(
        player,
        <ClipPlayer song={song} onClose={vi.fn()} session={carried} />,
      ),
    );
    clickBack();
    expect(player.toggle).not.toHaveBeenCalled();
  });

  it("leaves the track where it was when closed before the clip started", async () => {
    // The ticket is in, the element reads 0 and has played nothing.
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    const player = fakePlayer(false);
    await openClip(player);
    expect(videoElement().currentTime).toBe(0);
    clickBack();
    expect(player.seek).not.toHaveBeenCalled();
  });

  async function renewAfter(events: string[]) {
    clipUrl
      .mockResolvedValueOnce({ url: "/api/v2/clip-stream/one", expiresAt: 1 })
      .mockResolvedValueOnce({ url: "/api/v2/clip-stream/two", expiresAt: 2 });
    await openClip(fakePlayer(false));
    act(() => {
      for (const name of events) videoElement().dispatchEvent(new Event(name));
    });
    await act(async () => {
      videoElement().dispatchEvent(new Event("error"));
    });
    expect(videoElement().getAttribute("src")).toBe("/api/v2/clip-stream/two");
    return videoElement().autoplay;
  }

  it("stays paused through a renewed ticket after a pause", async () => {
    expect(await renewAfter(["play", "pause"])).toBe(false);
  });

  it("plays on through a renewed ticket while it plays", async () => {
    expect(await renewAfter(["play"])).toBe(true);
  });

  it("stays paused, and leaves the track alone, when it left paused", async () => {
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    const carried = session();
    const player = fakePlayer(false);
    await act(async () =>
      renderUnder(
        player,
        <ClipPlayer
          key="detail"
          song={song}
          onClose={vi.fn()}
          session={carried}
        />,
      ),
    );
    act(() => {
      videoElement().dispatchEvent(new Event("pause"));
    });

    // The track started from the bar while the clip was out of sight.
    player.playing = true;
    await act(async () =>
      renderUnder(
        player,
        <ClipPlayer
          key="immersive"
          song={song}
          onClose={vi.fn()}
          session={carried}
        />,
      ),
    );
    expect(videoElement().autoplay).toBe(false);
    expect(player.toggle).not.toHaveBeenCalled();
    expect(player.playing).toBe(true);
  });
});

describe("ClipDialog", () => {
  beforeEach(() => {
    // jsdom has the element but not the top layer.
    HTMLDialogElement.prototype.showModal = function (this: HTMLDialogElement) {
      this.open = true;
    };
    HTMLDialogElement.prototype.close = function (this: HTMLDialogElement) {
      this.open = false;
    };
  });

  it("gives the track back when Escape closes it", async () => {
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    const player = fakePlayer(true);
    player.current = { id: "another" };
    const onClose = vi.fn();
    await act(async () =>
      renderUnder(player, <ClipDialog song={song} onClose={onClose} />),
    );
    expect(player.playing).toBe(false);

    const cancel = new Event("cancel", { cancelable: true });
    act(() => {
      container.querySelector("dialog")?.dispatchEvent(cancel);
    });
    expect(cancel.defaultPrevented).toBe(true);
    expect(player.playing).toBe(true);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("is shut before the caller gives focus back", async () => {
    // The caller focuses the row behind, which is inert while the dialog is
    // still modal: the dialog has to be closed by the time it is told.
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    const openWhenTold: boolean[] = [];
    const onClose = vi.fn(() => {
      openWhenTold.push(Boolean(container.querySelector("dialog")?.open));
    });
    await act(async () =>
      renderUnder(
        fakePlayer(false),
        <ClipDialog song={song} onClose={onClose} />,
      ),
    );
    clickBack();
    expect(openWhenTold).toEqual([false]);
  });

  it("takes a selection released on the backdrop for no click outside", async () => {
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    const onClose = vi.fn();
    await act(async () =>
      renderUnder(
        fakePlayer(false),
        <ClipDialog song={song} onClose={onClose} />,
      ),
    );
    const dialog = container.querySelector("dialog") as HTMLDialogElement;
    const title = container.querySelector("header strong") as HTMLElement;

    // Pressed in the title, released outside: the click lands on the dialog.
    act(() => {
      title.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
      dialog.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onClose).not.toHaveBeenCalled();

    // Pressed and released on the backdrop.
    act(() => {
      dialog.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
      dialog.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onClose).toHaveBeenCalledTimes(1);
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

/** The playing screen, as far as the clip goes: one opening, shown in a view. */
function Screen({ trackId, view }: { trackId: string; view: string }) {
  const clip = useClipOpen(trackId, view !== "lyrics");
  return clip.open && view !== "lyrics" ? (
    <ClipPlayer
      key={view}
      song={{ ...song, id: trackId }}
      onClose={clip.hide}
      session={clip.session}
    />
  ) : (
    <button type="button" onClick={clip.show}>
      Watch
    </button>
  );
}

/**
 * The clip goes away without being closed: the screen is left, the view
 * changes, the track changes, or the menu's dialog is taken down by Back.
 */
describe("a clip that goes without being closed", () => {
  async function openOnScreen(player: ReturnType<typeof fakePlayer>) {
    act(() => renderUnder(player, <Screen trackId="track" view="detail" />));
    await act(async () => container.querySelector("button")?.click());
    expect(player.playing).toBe(false);
  }

  it("gives the track back when the playing screen is left", async () => {
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    const player = fakePlayer(true);
    await openOnScreen(player);
    playTo(60);

    act(() => renderUnder(player, null));
    expect(player.seek).toHaveBeenCalledWith(60);
    expect(player.playing).toBe(true);
  });

  it("gives nothing back on a change of view, which is the clip moving", async () => {
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    const player = fakePlayer(true);
    await openOnScreen(player);
    await act(async () =>
      renderUnder(player, <Screen trackId="track" view="immersive" />),
    );
    expect(player.toggle).toHaveBeenCalledTimes(1);
    expect(player.playing).toBe(false);
    expect(container.querySelector("video")).not.toBeNull();
  });

  // Two guards hold this, and removing either alone leaves it green: the
  // change of track starts a fresh session no player has claimed, and the
  // player gives back only the track it paused (`openedOn`).
  it("gives nothing back when the track changes", async () => {
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    const player = fakePlayer(true);
    await openOnScreen(player);
    player.current = { id: "next" };
    act(() => renderUnder(player, <Screen trackId="next" view="detail" />));
    act(() => renderUnder(player, null));
    expect(player.toggle).toHaveBeenCalledTimes(1);
  });

  it("gives the track back when the menu's dialog is taken down", async () => {
    HTMLDialogElement.prototype.showModal = function (this: HTMLDialogElement) {
      this.open = true;
    };
    HTMLDialogElement.prototype.close = function (this: HTMLDialogElement) {
      this.open = false;
    };
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    const player = fakePlayer(true);
    player.current = { id: "another" };
    await act(async () =>
      renderUnder(player, <ClipDialog song={song} onClose={vi.fn()} />),
    );
    expect(player.playing).toBe(false);

    // Back, from behind a modal dialog: the page goes, the dialog with it.
    act(() => renderUnder(player, null));
    expect(player.playing).toBe(true);
  });

  it("gives the track back once, when the button closed it first", async () => {
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    const player = fakePlayer(true);
    await openClip(player);
    clickBack();
    expect(player.toggle).toHaveBeenCalledTimes(2);
    // In the browser the dialog goes in the same update, and the player the
    // cleanup reads is the last one rendered, which still says paused: a
    // second giving back would toggle the track straight off again.
    player.playing = false;
    act(() => renderUnder(player, null));
    expect(player.toggle).toHaveBeenCalledTimes(2);
  });

  it("closes on the lyrics view, so leaving later moves nothing", async () => {
    clipUrl.mockResolvedValue({ url: "/api/v2/clip-stream/one", expiresAt: 1 });
    const player = fakePlayer(true);
    await openOnScreen(player);
    playTo(30);

    // Turning to the words closes the clip as its button would.
    act(() => renderUnder(player, <Screen trackId="track" view="lyrics" />));
    expect(player.seek).toHaveBeenCalledWith(30);
    expect(player.playing).toBe(true);
    expect(player.toggle).toHaveBeenCalledTimes(2);

    // The listener pauses and plays from the bar, then leaves the page: the
    // track is theirs now, and nothing sends it back or toggles it off.
    player.seek.mockClear();
    act(() => renderUnder(player, null));
    expect(player.seek).not.toHaveBeenCalled();
    expect(player.toggle).toHaveBeenCalledTimes(2);
    expect(player.playing).toBe(true);
  });
});
