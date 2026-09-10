import { Link } from "@tanstack/react-router";
import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  artworkUrl,
  formatDuration,
  getQueue,
  type Song,
  saveQueue,
  scrobble,
  streamUrl,
} from "./api";

import { Artwork } from "./artwork";
import { useI18n } from "./i18n";
import { Icon } from "./icons";

const VOLUME_KEY = "waveflow.volume";

/**
 * The last level this browser was left at. Per-viewer and per-device on
 * purpose: how loud a laptop should be is not a property of the account.
 */
export function readStoredVolume(): number {
  try {
    // `Number(null)` is 0, and so is `Number("")`. Converting before checking
    // that anything was stored started every fresh browser silent, at a level
    // nobody had chosen and with the mute button reporting nothing was muted.
    const raw = localStorage.getItem(VOLUME_KEY);
    if (raw === null || raw.trim() === "") return 1;
    const saved = Number(raw);
    if (Number.isFinite(saved) && saved >= 0 && saved <= 1) return saved;
  } catch {
    // Private windows and blocked site data both land here; full volume is a
    // safe answer and the session still works without persistence.
  }
  return 1;
}

export type RepeatMode = "off" | "all" | "one";

/**
 * The order playback walks, as positions into the queue.
 *
 * Shuffling is a *reading order* and not a rearrangement of the queue: the
 * queue page keeps showing what was queued, in the order it was queued, and
 * turning shuffle off resumes the line where it left it. Shuffling the array
 * itself would lose that, and would make "add to queue" land somewhere the
 * listener did not choose.
 *
 * The current position leads, so enabling shuffle never interrupts what is
 * playing.
 */
export function shuffledOrder(
  length: number,
  current: number,
  random: () => number = Math.random,
): number[] {
  const rest = Array.from({ length }, (_, index) => index).filter(
    (index) => index !== current,
  );
  for (let i = rest.length - 1; i > 0; i--) {
    const j = Math.floor(random() * (i + 1));
    [rest[i], rest[j]] = [rest[j] as number, rest[i] as number];
  }
  return current >= 0 && current < length ? [current, ...rest] : rest;
}

/**
 * Where playback goes after `current`, or `null` to stop there.
 *
 * `automatic` says whether the track ended by itself or the listener pressed
 * next, and it is the whole of what separates them: `repeat: "one"` replays a
 * track that ended, but pressing next under it still moves on. A next button
 * that did nothing would look broken.
 */
export function advance(
  order: number[],
  current: number,
  repeat: RepeatMode,
  automatic: boolean,
): number | null {
  if (repeat === "one" && automatic) return current;
  const at = order.indexOf(current);
  if (at === -1) return null;
  const following = order[at + 1];
  if (following !== undefined) return following;
  return repeat === "all" ? (order[0] ?? null) : null;
}

/**
 * Whether the queue an order was drawn for is still the queue being played,
 * and where its entries went.
 *
 * `false` means a different queue and a fresh draw. Anything else is the
 * mapping `followQueue` needs — `moved[before]` is where that entry sits now,
 * or -1 if it is gone. An append and a truncation are the identity mapping,
 * with the entries that went away pointing past the end.
 *
 * Compared by entry identity, not by length or by prefix. Length alone let one
 * five-track album inherit another's draw; a prefix test refused every removal,
 * including one from the middle, and reshuffled the rest of the listening under
 * the listener.
 */
export function continuationOf<T>(
  drawn: { entries: readonly T[]; shuffle: boolean } | null,
  entries: readonly T[],
  shuffle: boolean,
): false | number[] {
  if (drawn === null || drawn.shuffle !== shuffle) return false;
  const moved = drawn.entries.map((entry) => entries.indexOf(entry));
  return moved.some((position) => position >= 0) ? moved : false;
}

/**
 * Re-points a reading order at a queue whose entries have moved.
 *
 * `moved[before]` is where the entry that was at `before` sits now, or -1 if it
 * is gone. Removing a track from the middle shifts everything after it down by
 * one, so an order made of positions stops meaning what it meant — following
 * the numbers alone would keep playing, but not the songs that were chosen.
 */
export function followQueue(
  previous: number[],
  moved: readonly number[],
  length: number,
): number[] {
  const kept: number[] = [];
  const seen = new Set<number>();
  for (const before of previous) {
    const now = moved[before];
    if (now === undefined || now < 0 || now >= length || seen.has(now))
      continue;
    kept.push(now);
    seen.add(now);
  }
  for (let position = 0; position < length; position++) {
    if (!seen.has(position)) kept.push(position);
  }
  return kept;
}

/**
 * The reading order a queue should have, given the one it had before.
 *
 * `continues` says whether this is the same listening session rather than a
 * different queue altogether. It cannot be inferred from the length: replacing
 * a five-track album with another five-track album left the previous draw in
 * place, and a starting position that fell at the end of that draw stopped
 * playback after one track.
 *
 * `continues` is `false` for a different queue, or the mapping saying where the
 * previous queue's entries went. There is no separate "kept their positions"
 * case: an append and a truncation are the identity mapping, and `followQueue`
 * reads them the same way it reads a removal from the middle.
 */
export function orderForQueue(
  length: number,
  at: number,
  shuffle: boolean,
  previous: number[],
  continues: false | readonly number[],
  random: () => number = Math.random,
): number[] {
  if (!shuffle) return Array.from({ length }, (_, position) => position);
  if (continues === false || previous.length === 0) {
    return shuffledOrder(length, at, random);
  }
  return followQueue(previous, continues, length);
}

/** Where "previous" goes. Wraps only when the whole queue repeats. */
export function retreat(
  order: number[],
  current: number,
  repeat: RepeatMode,
): number | null {
  const at = order.indexOf(current);
  if (at === -1) return null;
  if (at > 0) return order[at - 1] ?? null;
  return repeat === "all" ? (order[order.length - 1] ?? null) : null;
}

type PlayerState = {
  queue: Song[];
  index: number;
  current: Song | null;
  playing: boolean;
  error: boolean;
  play: (queue: Song[], index: number) => void;
  /** Appends to the end of the queue without disturbing what is playing. */
  enqueue: (songs: Song[]) => void;
  shuffle: boolean;
  toggleShuffle: () => void;
  repeat: RepeatMode;
  cycleRepeat: () => void;
  /** 0 to 1. Muting keeps the level, so unmuting returns to it. */
  volume: number;
  setVolume: (volume: number) => void;
  muted: boolean;
  toggleMute: () => void;
  remove: (index: number) => void;
  clear: () => void;
  toggle: () => void;
  next: () => void;
  previous: () => void;
  seek: (seconds: number) => void;
};

type PlayerProgress = {
  position: number;
  duration: number;
};

const PlayerContext = createContext<PlayerState | null>(null);
const PlayerProgressContext = createContext<PlayerProgress | null>(null);

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

export function usePlayer(): PlayerState {
  const player = useContext(PlayerContext);
  if (!player) throw new Error("usePlayer requires PlayerProvider");
  return player;
}

/** Position and duration in seconds, ticking as the element plays. */
export function usePlayerProgress(): PlayerProgress {
  const progress = useContext(PlayerProgressContext);
  if (!progress) throw new Error("usePlayerProgress requires PlayerProvider");
  return progress;
}

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

  const resolveStream = useCallback(async (trackId: string) => {
    const cached = streamUrls.current.get(trackId);
    if (cached && cached.expiresAt > Date.now() + 5_000) return cached.url;
    const ticket = await streamUrl(trackId);
    streamUrls.current.set(trackId, ticket);
    return ticket.url;
  }, []);

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

  useEffect(() => {
    if (!audio.current) {
      audio.current = new Audio();
      audio.current.preload = "auto";
    }
    const element = audio.current;
    const onTime = () => setPosition(element.currentTime);
    const onDuration = () => setDuration(element.duration || 0);
    // Stop on the last track rather than stepping past it: an out-of-range
    // index empties `current` and the player bar vanishes mid-listen.
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
        void element.play().catch(() => undefined);
        return;
      }
      autoplay.current = true;
      setIndex(target);
    };
    const onPlay = () => {
      suppressedPauseEvents.current = 0;
      setPlaying(true);
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
  }, [hydrated, persistQueue]);

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
  }, [current, resolveStream]);

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
  }, [queue, index, resolveStream]);

  useEffect(() => {
    if (!current || duration <= 0) return;
    if (submitted.current === current.id) return;
    if (position > duration / 2) {
      submitted.current = current.id;
      void scrobble(current.id, true).catch(() => undefined);
    }
  }, [position, duration, current]);

  const play = useCallback((next: Song[], at: number) => {
    localMutation.current = true;
    const sameSelection = next === queueRef.current && at === indexRef.current;
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
  }, []);

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
    void element
      .play()
      .then(() => scrobble(current.id, false))
      .catch(() => undefined);
  }, [current]);

  const pausePlayback = useCallback(() => {
    const element = audio.current;
    if (element && !element.paused) element.pause();
  }, []);

  // The element is the authority on level; state mirrors it for the control.
  useEffect(() => {
    const element = audio.current;
    if (!element) return;
    element.volume = volume;
    element.muted = muted;
  }, [volume, muted]);

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

  const next = useCallback(() => {
    localMutation.current = true;
    setIndex((value) => {
      // Pressed, not ended: `repeat: "one"` must not trap the button here.
      const target = advance(orderRef.current, value, repeatRef.current, false);
      if (target === null) return value;
      autoplay.current = target !== value;
      return target;
    });
  }, []);

  const previous = useCallback(() => {
    localMutation.current = true;
    setIndex((value) => {
      const target = retreat(orderRef.current, value, repeatRef.current);
      if (target === null) return value;
      autoplay.current = target !== value;
      return target;
    });
  }, []);

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

  useEffect(() => {
    const mediaSession = navigator.mediaSession;
    const element = audio.current;
    if (!mediaSession || !element) return;
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
  }, [next, pausePlayback, previous, seek, startPlayback]);

  useEffect(() => {
    if (!navigator.mediaSession) return;
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
  }, [current, playing, t]);

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
      if (event.code === "Space") {
        event.preventDefault();
        toggle();
      } else if (event.code === "ArrowRight") {
        event.preventDefault();
        seek(positionRef.current + 5);
      } else if (event.code === "ArrowLeft") {
        event.preventDefault();
        seek(positionRef.current - 5);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [seek, toggle]);

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
    }),
    [
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

  return (
    <PlayerContext.Provider value={value}>
      <PlayerProgressContext.Provider value={progress}>
        {children}
      </PlayerProgressContext.Provider>
    </PlayerContext.Provider>
  );
}

/** Render the persistent controls for the currently selected track. */
export function PlayerBar() {
  const player = usePlayer();
  const progress = usePlayerProgress();
  const { t } = useI18n();
  const [scrubbing, setScrubbing] = useState<number | null>(null);
  if (!player.current) return null;
  const commit = (value: number) => {
    player.seek(value);
    setScrubbing(null);
  };
  const cover = (
    <Artwork
      artworkId={player.current.artwork_hash}
      title={player.current.title}
      className="player-cover"
    />
  );
  const repeatLabel =
    player.repeat === "off"
      ? t("player.repeatOff")
      : player.repeat === "all"
        ? t("player.repeatAll")
        : t("player.repeatOne");
  return (
    <footer className="player">
      {/* The cover is the way back to what is playing. Without it the only
          route to the album of the current track is to remember its name and
          search for it. */}
      {player.current.album_id ? (
        <Link
          to="/albums/$albumId"
          params={{ albumId: player.current.album_id }}
          aria-label={`${t("player.openAlbum")}: ${player.current.album ?? player.current.title}`}
        >
          {cover}
        </Link>
      ) : (
        cover
      )}
      <div className="player-track">
        <strong>{player.current.title}</strong>
        <span>{player.current.artist ?? t("common.unknownArtist")}</span>
        {player.error ? <small role="alert">{t("player.error")}</small> : null}
      </div>
      <div className="player-controls">
        <button
          type="button"
          className={player.shuffle ? "mode on" : "mode"}
          onClick={player.toggleShuffle}
          aria-label={t("player.shuffle")}
          aria-pressed={player.shuffle}
        >
          <Icon name="random" size={18} />
        </button>
        <button
          type="button"
          onClick={player.previous}
          aria-label={t("player.previous")}
        >
          <Icon name="previous" />
        </button>
        <button
          type="button"
          className="primary"
          onClick={player.toggle}
          aria-label={player.playing ? t("player.pause") : t("player.play")}
        >
          <Icon name={player.playing ? "pause" : "play"} size={24} />
        </button>
        <button
          type="button"
          onClick={player.next}
          aria-label={t("player.next")}
        >
          <Icon name="next" />
        </button>
        {/* Three states on one button, so its label carries the current one
            rather than the action — a screen reader hearing "repeat" alone
            could not tell which of the three is on. */}
        <button
          type="button"
          className={player.repeat === "off" ? "mode" : "mode on"}
          onClick={player.cycleRepeat}
          aria-label={repeatLabel}
        >
          <Icon
            name={player.repeat === "one" ? "repeatOne" : "repeat"}
            size={18}
          />
        </button>
      </div>
      <div className="player-progress">
        <span>{formatDuration((scrubbing ?? progress.position) * 1000)}</span>
        <input
          type="range"
          min={0}
          max={progress.duration || 0}
          step={0.5}
          value={scrubbing ?? progress.position}
          onChange={(event) => setScrubbing(Number(event.target.value))}
          onMouseUp={(event) => commit(Number(event.currentTarget.value))}
          onTouchEnd={(event) => commit(Number(event.currentTarget.value))}
          onKeyUp={(event) => commit(Number(event.currentTarget.value))}
          aria-label={t("player.seek")}
        />
        <span>{formatDuration(progress.duration * 1000)}</span>
      </div>
      <div className="player-aside">
        <button
          type="button"
          onClick={player.toggleMute}
          aria-label={player.muted ? t("player.unmute") : t("player.mute")}
          aria-pressed={player.muted}
        >
          <Icon name={player.muted ? "muted" : "volume"} size={18} />
        </button>
        <input
          type="range"
          className="volume"
          min={0}
          max={1}
          step={0.01}
          value={player.muted ? 0 : player.volume}
          onChange={(event) => player.setVolume(Number(event.target.value))}
          aria-label={t("player.volume")}
        />
        <Link className="nav-action" to="/queue">
          <Icon name="queue" size={18} />
          <span>{t("nav.queue")}</span>
        </Link>
      </div>
    </footer>
  );
}
