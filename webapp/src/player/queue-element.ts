// The queue's element: its events, loading, preloading and the scrobble.
import {
  type Dispatch,
  type RefObject,
  type SetStateAction,
  useEffect,
} from "react";

import { type Song, scrobble } from "../api";
import type { Aside } from "../lib/aside";
import { advance, type RepeatMode } from "./order";

/** The provider's state and refs this part of it reads and writes. */
type QueueElement = {
  audio: RefObject<HTMLAudioElement | null>;
  preloader: RefObject<HTMLAudioElement | null>;
  streamUrls: RefObject<Map<string, { url: string; expiresAt: number }>>;
  queue: Song[];
  index: number;
  current: Song | null;
  position: number;
  duration: number;
  hydrated: boolean;
  setIndex: Dispatch<SetStateAction<number>>;
  setPlaying: Dispatch<SetStateAction<boolean>>;
  setPosition: Dispatch<SetStateAction<number>>;
  setDuration: Dispatch<SetStateAction<number>>;
  setPlaybackError: Dispatch<SetStateAction<boolean>>;
  queueRef: RefObject<Song[]>;
  indexRef: RefObject<number>;
  orderRef: RefObject<number[]>;
  repeatRef: RefObject<RepeatMode>;
  positionRef: RefObject<number>;
  localMutation: RefObject<boolean>;
  submitted: RefObject<string | null>;
  autoplay: RefObject<boolean>;
  suppressedPauseEvents: RefObject<number>;
  resumePosition: RefObject<number>;
  resumeTrack: RefObject<string | null>;
  asideRef: RefObject<Aside | null>;
  persistQueue: (
    songs: Song[],
    selected: string | null,
    positionMs: number,
  ) => void;
  stopAside: (only?: string) => void;
  wakeGraph: () => void;
  resolveStream: (trackId: string) => Promise<string>;
};

/** Part of `PlayerProvider`, called in its place among the provider's hooks. */
export function useQueueElement({
  audio,
  preloader,
  streamUrls,
  queue,
  index,
  current,
  position,
  duration,
  hydrated,
  setIndex,
  setPlaying,
  setPosition,
  setDuration,
  setPlaybackError,
  queueRef,
  indexRef,
  orderRef,
  repeatRef,
  positionRef,
  localMutation,
  submitted,
  autoplay,
  suppressedPauseEvents,
  resumePosition,
  resumeTrack,
  asideRef,
  persistQueue,
  stopAside,
  wakeGraph,
  resolveStream,
}: QueueElement): void {
  useEffect(() => {
    if (!audio.current) {
      audio.current = new Audio();
      audio.current.preload = "auto";
    }
    const element = audio.current;
    const onTime = () => setPosition(element.currentTime);
    const onDuration = () => setDuration(element.duration || 0);
    const onEnd = () => {
      // Decided out here rather than inside a `setIndex` updater: React may
      // call an updater more than once, and this one rewound and restarted the
      // element, which would then have happened twice.
      const from = indexRef.current;
      const target = advance(orderRef.current, from, repeatRef.current, true);
      // Nothing follows: stay on the last track rather than stepping past it,
      // because an out-of-range index empties `current` and the player bar
      // vanishes mid-listen.
      if (target === null) return;
      localMutation.current = true;
      if (target === from) {
        // Repeat-one lands on the same index, which changes no state and so
        // would not restart the element. Rewind and play it again by hand.
        element.currentTime = 0;
        submitted.current = null;
        wakeGraph();
        void element.play().catch(() => undefined);
        return;
      }
      autoplay.current = true;
      setIndex(target);
    };
    const onPlay = () => {
      suppressedPauseEvents.current = 0;
      setPlaying(true);
      // A track sounding takes over from a station or an episode, however it
      // was started: there is only ever one audible source.
      stopAside();
    };
    const onPause = () => {
      setPlaying(false);
      if (suppressedPauseEvents.current > 0) {
        suppressedPauseEvents.current -= 1;
        return;
      }
      if (!hydrated) return;
      const songs = queueRef.current;
      const selected = songs[indexRef.current] ?? null;
      persistQueue(
        songs,
        selected?.id ?? null,
        Math.round(positionRef.current * 1000),
      );
    };
    element.addEventListener("timeupdate", onTime);
    element.addEventListener("loadedmetadata", onDuration);
    element.addEventListener("ended", onEnd);
    element.addEventListener("play", onPlay);
    element.addEventListener("pause", onPause);
    return () => {
      element.removeEventListener("timeupdate", onTime);
      element.removeEventListener("loadedmetadata", onDuration);
      element.removeEventListener("ended", onEnd);
      element.removeEventListener("play", onPlay);
      element.removeEventListener("pause", onPause);
    };
  }, [
    hydrated,
    persistQueue,
    stopAside,
    wakeGraph,
    audio,
    autoplay,
    indexRef,
    localMutation,
    orderRef,
    positionRef,
    queueRef,
    repeatRef,
    setDuration,
    setIndex,
    setPlaying,
    setPosition,
    submitted,
    suppressedPauseEvents,
  ]);

  // Loading a track needs a round-trip for its ticket, so guard against a
  // stale response overwriting a newer selection.
  useEffect(() => {
    const element = audio.current;
    const resumeSeconds =
      current && resumeTrack.current === current.id
        ? resumePosition.current
        : 0;
    positionRef.current = resumeSeconds;
    setPosition(resumeSeconds);
    setDuration(0);
    setPlaybackError(false);
    if (!element) return;
    suppressedPauseEvents.current = 2;
    element.pause();
    element.removeAttribute("src");
    element.load();
    if (!current) return;
    let cancelled = false;
    submitted.current = null;
    const shouldAutoplay = autoplay.current;
    autoplay.current = false;
    const onError = () => {
      if (cancelled) return;
      streamUrls.current.delete(current.id);
      element.pause();
      setPlaying(false);
      setPlaybackError(true);
    };
    element.addEventListener("error", onError);
    void (async () => {
      try {
        const url = await resolveStream(current.id);
        if (cancelled) return;
        element.src = url;
        if (resumeSeconds > 0) {
          element.currentTime = resumeSeconds;
        }
        if (resumeTrack.current === current.id) {
          resumePosition.current = 0;
          resumeTrack.current = null;
        }
        if (!shouldAutoplay) return;
        // A station or an episode started while this ticket was on its way
        // is the later choice, and keeps the speakers.
        if (asideRef.current) return;
        wakeGraph();
        await element.play();
        void scrobble(current.id, false).catch(() => undefined);
      } catch {
        streamUrls.current.delete(current.id);
        if (!cancelled) {
          setPlaying(false);
          setPlaybackError(true);
        }
      }
    })();
    return () => {
      cancelled = true;
      element.removeEventListener("error", onError);
    };
  }, [
    current,
    resolveStream,
    wakeGraph,
    audio,
    asideRef,
    autoplay,
    positionRef,
    resumePosition,
    resumeTrack,
    setDuration,
    setPlaybackError,
    setPlaying,
    setPosition,
    streamUrls,
    submitted,
    suppressedPauseEvents,
  ]);

  useEffect(() => {
    const upcoming = queue[index + 1];
    if (!upcoming) return;
    let cancelled = false;
    void resolveStream(upcoming.id)
      .then((url) => {
        if (cancelled) return;
        const element = preloader.current ?? new Audio();
        preloader.current = element;
        element.preload = "metadata";
        element.src = url;
        element.load();
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [queue, index, resolveStream, preloader]);

  useEffect(() => {
    if (!current || duration <= 0) return;
    if (submitted.current === current.id) return;
    if (position > duration / 2) {
      submitted.current = current.id;
      void scrobble(current.id, true).catch(() => undefined);
    }
  }, [position, duration, current, submitted]);
}
