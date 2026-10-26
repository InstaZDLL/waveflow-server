// The system's media controls and the keyboard, driving the player.
import { type RefObject, useEffect } from "react";

import { artworkUrl, cachedArtworkUrl, type Song } from "../api";
import {
  type Aside,
  asideArtwork,
  asideMetadata,
  asideTransport,
  sounding,
} from "../lib/aside";
import type { useI18n } from "../state/i18n";

export function setDirectionalMediaSessionHandlers(
  mediaSession: Pick<MediaSession, "setActionHandler">,
  element: Pick<HTMLAudioElement, "paused">,
  play: () => void,
  pause: () => void,
): void {
  mediaSession.setActionHandler("play", () => {
    if (element.paused) play();
  });
  mediaSession.setActionHandler("pause", () => {
    if (!element.paused) pause();
  });
}

/**
 * The same directional handlers for the aside, which ask whether it is
 * sounding rather than whether it is paused: a failed station keeps `paused`
 * false, and the system's Play would be ignored and its Pause would retry it.
 * The queue's element keeps the plain rule above.
 */
export function setAsideMediaSessionHandlers(
  mediaSession: Pick<MediaSession, "setActionHandler">,
  element: Pick<HTMLAudioElement, "paused" | "error">,
  toggle: () => void,
): void {
  setDirectionalMediaSessionHandlers(
    mediaSession,
    {
      get paused() {
        return !sounding(element);
      },
    },
    toggle,
    toggle,
  );
}

/** The provider's state, refs and controls this part of it reads. */
type SystemControls = {
  audio: RefObject<HTMLAudioElement | null>;
  side: RefObject<HTMLAudioElement | null>;
  aside: Aside | null;
  asidePlaying: boolean;
  asideRef: RefObject<Aside | null>;
  asidePositionRef: RefObject<number>;
  current: Song | null;
  playing: boolean;
  positionRef: RefObject<number>;
  t: ReturnType<typeof useI18n>["t"];
  next: () => void;
  previous: () => void;
  seek: (seconds: number) => void;
  seekAside: (seconds: number) => void;
  startPlayback: () => void;
  pausePlayback: () => void;
  toggle: () => void;
  toggleAside: () => void;
};

/** Part of `PlayerProvider`, called in its place among the provider's hooks. */
export function useSystemControls({
  audio,
  side,
  aside,
  asidePlaying,
  asideRef,
  asidePositionRef,
  current,
  playing,
  positionRef,
  t,
  next,
  previous,
  seek,
  seekAside,
  startPlayback,
  pausePlayback,
  toggle,
  toggleAside,
}: SystemControls): void {
  // The system's controls drive whatever the bar shows: the queue, or the
  // aside — which has no next or previous, and a station nowhere to seek.
  const asideKind = aside?.kind ?? null;
  useEffect(() => {
    const mediaSession = navigator.mediaSession;
    const element = asideKind ? side.current : audio.current;
    if (!mediaSession || !element) return;
    if (asideKind) {
      setAsideMediaSessionHandlers(mediaSession, element, toggleAside);
      mediaSession.setActionHandler("previoustrack", null);
      mediaSession.setActionHandler("nexttrack", null);
      mediaSession.setActionHandler(
        "seekto",
        asideTransport(asideKind).seek
          ? (details) => {
              if (details.seekTime !== undefined) seekAside(details.seekTime);
            }
          : null,
      );
    } else {
      setDirectionalMediaSessionHandlers(
        mediaSession,
        element,
        startPlayback,
        pausePlayback,
      );
      mediaSession.setActionHandler("previoustrack", previous);
      mediaSession.setActionHandler("nexttrack", next);
      mediaSession.setActionHandler("seekto", (details) => {
        if (details.seekTime !== undefined) seek(details.seekTime);
      });
    }
    return () => {
      for (const action of [
        "play",
        "pause",
        "previoustrack",
        "nexttrack",
        "seekto",
      ] as MediaSessionAction[]) {
        mediaSession.setActionHandler(action, null);
      }
    };
  }, [
    asideKind,
    next,
    pausePlayback,
    previous,
    seek,
    seekAside,
    startPlayback,
    toggleAside,
    audio,
    side,
  ]);

  useEffect(() => {
    if (!navigator.mediaSession) return;
    if (aside) {
      navigator.mediaSession.playbackState = asidePlaying
        ? "playing"
        : "paused";
      const metadata = asideMetadata(aside, {
        live: t("player.liveRadio"),
        podcasts: t("nav.podcasts"),
      });
      // Shown at once, with the cover if it is already held — this runs again
      // on every pause and play — and otherwise again once its object URL is
      // ready: the system's controls should not wait on an image.
      const cover = asideArtwork(aside);
      const held = cachedArtworkUrl(cover);
      navigator.mediaSession.metadata = new MediaMetadata({
        ...metadata,
        artwork: held ? [{ src: held }] : [],
      });
      if (held || !cover) return;
      let cancelled = false;
      void artworkUrl(cover).then((url) => {
        if (cancelled || !url) return;
        navigator.mediaSession.metadata = new MediaMetadata({
          ...metadata,
          artwork: [{ src: url }],
        });
      });
      return () => {
        cancelled = true;
      };
    }
    navigator.mediaSession.playbackState = playing ? "playing" : "paused";
    if (!current) {
      navigator.mediaSession.metadata = null;
      return;
    }
    let cancelled = false;
    void artworkUrl(current.artwork_hash).then((url) => {
      if (cancelled) return;
      navigator.mediaSession.metadata = new MediaMetadata({
        title: current.title,
        artist: current.artist ?? t("common.unknownArtist"),
        album: current.album ?? "WaveFlow",
        artwork: url ? [{ src: url }] : [],
      });
    });
    return () => {
      cancelled = true;
    };
  }, [aside, asidePlaying, current, playing, t]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (
        event.defaultPrevented ||
        event.ctrlKey ||
        event.metaKey ||
        event.altKey ||
        target?.isContentEditable ||
        ["INPUT", "TEXTAREA", "SELECT"].includes(target?.tagName ?? "")
      ) {
        return;
      }
      // The keys act on what the bar shows, as its buttons do.
      const showing = asideRef.current;
      if (event.code === "Space") {
        event.preventDefault();
        if (showing) toggleAside();
        else toggle();
      } else if (event.code === "ArrowRight" || event.code === "ArrowLeft") {
        const step = event.code === "ArrowRight" ? 5 : -5;
        if (showing && !asideTransport(showing.kind).seek) return;
        event.preventDefault();
        if (showing) seekAside(asidePositionRef.current + step);
        else seek(positionRef.current + step);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [
    seek,
    seekAside,
    toggle,
    toggleAside,
    asideRef,
    asidePositionRef,
    positionRef,
  ]);
}
