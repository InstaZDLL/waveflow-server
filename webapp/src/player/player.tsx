// The provider that owns playback, and the face the client imports it by.
import {
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import { getQueue, type Song, saveQueue, scrobble, streamUrl } from "../api";
import { isKeptOffline, keptSongs, offlineTrackUrl } from "../lib/offline";
import { replayGainFactor } from "../lib/replay-gain";
import { useI18n } from "../state/i18n";
import { useAsidePlayback } from "./aside-playback";
import {
  AsideProgressContext,
  PlayerContext,
  PlayerProgressContext,
  type PlayerState,
} from "./context";
import {
  advance,
  continuationOf,
  orderForQueue,
  type RepeatMode,
  retreat,
} from "./order";
import { useQueueElement } from "./queue-element";
import { useReplayGainGraph } from "./replay-gain-graph";
import { useSystemControls } from "./system-controls";
import { readStoredVolume, VOLUME_KEY } from "./volume";

export { PlayerBar } from "./bar";
export { useAsideProgress, usePlayer, usePlayerProgress } from "./context";
export {
  advance,
  continuationOf,
  followQueue,
  orderForQueue,
  type RepeatMode,
  retreat,
  shuffledOrder,
} from "./order";
export {
  setAsideMediaSessionHandlers,
  setDirectionalMediaSessionHandlers,
} from "./system-controls";
export { readStoredVolume } from "./volume";

/** Own the shared audio element, queue, playback modes, and transport state. */
export function PlayerProvider({ children }: { children: ReactNode }) {
  const { t } = useI18n();
  const audio = useRef<HTMLAudioElement | null>(null);
  const [queue, setQueue] = useState<Song[]>([]);
  const [index, setIndex] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [position, setPosition] = useState(0);
  const [duration, setDuration] = useState(0);
  const [playbackError, setPlaybackError] = useState(false);
  const [shuffle, setShuffle] = useState(false);
  const [repeat, setRepeat] = useState<RepeatMode>("off");
  // The reading order, as queue positions. Identity while shuffle is off, so
  // the linear path costs nothing to express.
  const [order, setOrder] = useState<number[]>([]);
  const orderRef = useRef<number[]>([]);
  // What the current order was drawn for. Compared by array identity and by
  // mode, because neither the queue's length nor its contents alone say
  // whether this is the same listening session.
  const drawnFor = useRef<{ queue: Song[] | null; shuffle: boolean }>({
    queue: null,
    shuffle: false,
  });
  const repeatRef = useRef<RepeatMode>("off");
  const [volume, setVolumeState] = useState(() => readStoredVolume());
  const [muted, setMuted] = useState(false);
  // A completed listen is reported once per track, when playback passes half
  // of it — the same threshold the Subsonic clients use for a submission.
  const submitted = useRef<string | null>(null);
  // The ended handler is registered once, so it reads the queue length through
  // a ref rather than closing over a stale value.
  const queueLength = useRef(0);
  const queueRef = useRef<Song[]>([]);
  const indexRef = useRef(0);
  const positionRef = useRef(0);
  const [hydrated, setHydrated] = useState(false);
  const localMutation = useRef(false);
  const resumePosition = useRef(0);
  const resumeTrack = useRef<string | null>(null);
  const autoplay = useRef(false);
  const suppressedPauseEvents = useRef(0);
  const saveChain = useRef<Promise<void>>(Promise.resolve());
  const streamUrls = useRef(
    new Map<string, { url: string; expiresAt: number }>(),
  );
  const preloader = useRef<HTMLAudioElement | null>(null);
  const { replayGain, replayGainOn, gainNode, wakeGraph, prepareGraph } =
    useReplayGainGraph(audio, preloader);

  const current = queue[index] ?? null;
  orderRef.current =
    order.length === queue.length
      ? order
      : Array.from({ length: queue.length }, (_, position) => position);
  repeatRef.current = repeat;
  queueLength.current = queue.length;
  queueRef.current = queue;
  indexRef.current = index;
  positionRef.current = position;

  const persistQueue = useCallback(
    (songs: Song[], selected: string | null, positionMs: number) => {
      const snapshot = [...songs];
      saveChain.current = saveChain.current
        .then(() => saveQueue(snapshot, selected, positionMs))
        .catch(() => undefined);
    },
    [],
  );

  // What is kept is read once, so `isKeptOffline` can answer as a track
  // starts; keeping or forgetting one updates the set as it happens.
  useEffect(() => {
    void keptSongs().catch(() => undefined);
  }, []);

  const resolveStream = useCallback(async (trackId: string) => {
    // A kept track plays from this browser when the service worker answers
    // its address (RFC-022): no ticket, no network. Same origin, so the
    // ReplayGain graph hears it.
    if (isKeptOffline(trackId) && navigator.serviceWorker?.controller) {
      return offlineTrackUrl(trackId);
    }
    const cached = streamUrls.current.get(trackId);
    if (cached && cached.expiresAt > Date.now() + 5_000) return cached.url;
    const ticket = await streamUrl(trackId);
    streamUrls.current.set(trackId, ticket);
    return ticket.url;
  }, []);

  const {
    side,
    aside,
    asideRef,
    asidePlaying,
    asideError,
    asidePosition,
    asideDuration,
    asidePositionRef,
    stopAside,
    playLive,
    playEpisode,
    toggleAside,
    seekAside,
  } = useAsidePlayback(audio);

  useEffect(() => {
    let cancelled = false;
    void getQueue()
      .then((saved) => {
        if (cancelled || localMutation.current || !saved) return;
        setQueue(saved.songs);
        const savedIndex = saved.current
          ? saved.songs.findIndex((song) => song.id === saved.current)
          : 0;
        setIndex(Math.max(savedIndex, 0));
        resumePosition.current = Math.max(saved.position_ms, 0) / 1000;
        resumeTrack.current = saved.current;
      })
      // A transient queue failure must not become an unhandled browser error;
      // playback can still start a new queue and retry on its first mutation.
      .catch(() => undefined)
      .finally(() => {
        if (!cancelled) setHydrated(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useQueueElement({
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
  });

  const play = useCallback(
    (next: Song[], at: number) => {
      // First, inside the gesture that asked for the music.
      prepareGraph();
      stopAside();
      localMutation.current = true;
      const sameSelection =
        next === queueRef.current && at === indexRef.current;
      if (sameSelection) {
        const element = audio.current;
        const selected = next[at];
        if (element && selected) {
          void element
            .play()
            .then(() => scrobble(selected.id, false))
            .catch(() => undefined);
        }
        return;
      }
      autoplay.current = true;
      setQueue(next);
      setIndex(at);
    },
    [prepareGraph, stopAside],
  );

  useEffect(() => {
    if (!hydrated || !localMutation.current) return;
    const timeout = window.setTimeout(() => {
      persistQueue(
        queue,
        current?.id ?? null,
        Math.round(positionRef.current * 1000),
      );
    }, 400);
    return () => window.clearTimeout(timeout);
  }, [queue, current, hydrated, persistQueue]);

  const startPlayback = useCallback(() => {
    const element = audio.current;
    if (!element || !current || !element.paused) return;
    prepareGraph();
    void element
      .play()
      .then(() => scrobble(current.id, false))
      .catch(() => undefined);
  }, [current, prepareGraph]);

  // Turned on mid-track: the change came from the settings control, which is
  // a gesture, so the graph can be built while the browser still counts it as
  // one. Paused, it waits for the next play instead.
  useEffect(() => {
    if (replayGainOn && audio.current && !audio.current.paused) prepareGraph();
  }, [replayGainOn, prepareGraph]);

  // The level of the track now playing. Set when the track changes, before
  // its first sample, and again when the preference does; off is a gain of 1.
  useEffect(() => {
    if (!gainNode) return;
    const factor = current ? replayGainFactor(current, replayGain) : 1;
    gainNode.gain.setValueAtTime(factor, gainNode.context.currentTime);
  }, [gainNode, current, replayGain]);

  const pausePlayback = useCallback(() => {
    const element = audio.current;
    if (element && !element.paused) element.pause();
  }, []);

  // The element is the authority on level; state mirrors it for the control.
  // One level for both elements: the bar has one slider, whatever it plays.
  useEffect(() => {
    for (const element of [audio.current, side.current]) {
      if (!element) continue;
      element.volume = volume;
      element.muted = muted;
    }
  }, [volume, muted, side]);

  const setVolume = useCallback((next: number) => {
    const bounded = Math.max(0, Math.min(1, next));
    setVolumeState(bounded);
    // Touching the slider is how someone unmutes without hunting for the
    // button, so raising the level clears the mute.
    if (bounded > 0) setMuted(false);
    try {
      localStorage.setItem(VOLUME_KEY, String(bounded));
    } catch {
      // The level still applies for this session without persistence.
    }
  }, []);

  const toggleMute = useCallback(() => setMuted((value) => !value), []);

  const toggleShuffle = useCallback(() => setShuffle((on) => !on), []);

  const cycleRepeat = useCallback(
    () =>
      setRepeat((mode) =>
        mode === "off" ? "all" : mode === "all" ? "one" : "off",
      ),
    [],
  );

  // The reading order is owned here, in one place, and derived from the queue
  // and the mode rather than set from a toggle's updater — which is a side
  // effect in a place React is free to run twice.
  //
  // The queue is compared by identity and by prefix, not by length: replacing
  // one five-track album with another left the previous draw standing, and a
  // starting position that fell at the end of that draw stopped playback after
  // a single track.
  useEffect(() => {
    const drawn = drawnFor.current;
    const continues = continuationOf(
      drawn.queue === null
        ? null
        : { entries: drawn.queue, shuffle: drawn.shuffle },
      queue,
      shuffle,
    );
    drawnFor.current = { queue, shuffle };
    setOrder((current) =>
      orderForQueue(
        queue.length,
        indexRef.current,
        shuffle,
        current,
        continues,
      ),
    );
  }, [queue, shuffle]);

  const toggle = useCallback(() => {
    if (audio.current?.paused) startPlayback();
    else pausePlayback();
  }, [pausePlayback, startPlayback]);

  // Both are pressed, so both are a gesture the graph can be built in.
  const next = useCallback(() => {
    prepareGraph();
    stopAside();
    localMutation.current = true;
    setIndex((value) => {
      // Pressed, not ended: `repeat: "one"` must not trap the button here.
      const target = advance(orderRef.current, value, repeatRef.current, false);
      if (target === null) return value;
      autoplay.current = target !== value;
      return target;
    });
  }, [prepareGraph, stopAside]);

  const previous = useCallback(() => {
    prepareGraph();
    stopAside();
    localMutation.current = true;
    setIndex((value) => {
      const target = retreat(orderRef.current, value, repeatRef.current);
      if (target === null) return value;
      autoplay.current = target !== value;
      return target;
    });
  }, [prepareGraph, stopAside]);

  const seek = useCallback(
    (seconds: number) => {
      const element = audio.current;
      if (!element) return;
      localMutation.current = true;
      const bounded = Math.max(
        0,
        Math.min(seconds, element.duration || seconds),
      );
      element.currentTime = bounded;
      positionRef.current = bounded;
      setPosition(bounded);
      const songs = queueRef.current;
      const selected = songs[indexRef.current] ?? null;
      persistQueue(songs, selected?.id ?? null, Math.round(bounded * 1000));
    },
    [persistQueue],
  );

  useSystemControls({
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
  });

  const value = useMemo<PlayerState>(
    () => ({
      queue,
      index,
      current,
      playing,
      error: playbackError,
      play,
      // Appending to an empty queue selects the first song but does not start
      // it: `autoplay` stays false, so the source effect loads the track and
      // leaves it paused. "Add to queue" that began playing would be a
      // different button.
      enqueue: (songs: Song[]) => {
        if (!songs.length) return;
        localMutation.current = true;
        setQueue((current) => [...current, ...songs]);
      },
      remove: (at: number) => {
        localMutation.current = true;
        if (at === indexRef.current) {
          autoplay.current = false;
          audio.current?.pause();
        }
        setQueue((songs) => songs.filter((_, position) => position !== at));
        setIndex((value) =>
          value > at
            ? value - 1
            : Math.min(value, Math.max(queue.length - 2, 0)),
        );
      },
      clear: () => {
        localMutation.current = true;
        const element = audio.current;
        queueRef.current = [];
        indexRef.current = 0;
        positionRef.current = 0;
        if (element) {
          suppressedPauseEvents.current = 2;
          element.pause();
          element.removeAttribute("src");
          element.load();
        }
        autoplay.current = false;
        setQueue([]);
        setIndex(0);
        persistQueue([], null, 0);
      },
      toggle,
      next,
      previous,
      seek,
      shuffle,
      toggleShuffle,
      repeat,
      cycleRepeat,
      volume,
      setVolume,
      muted,
      toggleMute,
      aside,
      asidePlaying,
      asideError,
      playLive,
      playEpisode,
      toggleAside,
      seekAside,
      stopAside,
    }),
    [
      aside,
      asidePlaying,
      asideError,
      playLive,
      playEpisode,
      toggleAside,
      seekAside,
      stopAside,
      queue,
      index,
      current,
      playing,
      playbackError,
      play,
      toggle,
      next,
      previous,
      persistQueue,
      seek,
      shuffle,
      toggleShuffle,
      repeat,
      cycleRepeat,
      volume,
      setVolume,
      muted,
      toggleMute,
    ],
  );

  const progress = useMemo(
    () => ({ position, duration }),
    [position, duration],
  );

  const asideProgress = useMemo(
    () => ({ position: asidePosition, duration: asideDuration }),
    [asidePosition, asideDuration],
  );

  return (
    <PlayerContext.Provider value={value}>
      <PlayerProgressContext.Provider value={progress}>
        <AsideProgressContext.Provider value={asideProgress}>
          {children}
        </AsideProgressContext.Provider>
      </PlayerProgressContext.Provider>
    </PlayerContext.Provider>
  );
}
