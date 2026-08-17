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

type PlayerState = {
  queue: Song[];
  index: number;
  current: Song | null;
  playing: boolean;
  error: boolean;
  play: (queue: Song[], index: number) => void;
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

export function usePlayer(): PlayerState {
  const player = useContext(PlayerContext);
  if (!player) throw new Error("usePlayer requires PlayerProvider");
  return player;
}

function usePlayerProgress(): PlayerProgress {
  const progress = useContext(PlayerProgressContext);
  if (!progress) throw new Error("usePlayerProgress requires PlayerProvider");
  return progress;
}

export function PlayerProvider({ children }: { children: ReactNode }) {
  const { t } = useI18n();
  const audio = useRef<HTMLAudioElement | null>(null);
  const [queue, setQueue] = useState<Song[]>([]);
  const [index, setIndex] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [position, setPosition] = useState(0);
  const [duration, setDuration] = useState(0);
  const [playbackError, setPlaybackError] = useState(false);
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
  const streamUrls = useRef(new Map<string, string>());
  const preloader = useRef<HTMLAudioElement | null>(null);

  const current = queue[index] ?? null;
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
    if (cached) return cached;
    const url = await streamUrl(trackId);
    streamUrls.current.set(trackId, url);
    return url;
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
      localMutation.current = true;
      setIndex((value) => {
        const next = Math.min(value + 1, Math.max(queueLength.current - 1, 0));
        autoplay.current = next !== value;
        return next;
      });
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

  const toggle = useCallback(() => {
    const element = audio.current;
    if (!element || !current) return;
    if (element.paused) {
      void element
        .play()
        .then(() => scrobble(current.id, false))
        .catch(() => undefined);
    } else element.pause();
  }, [current]);

  const next = useCallback(() => {
    localMutation.current = true;
    setIndex((value) => {
      const next = Math.min(value + 1, Math.max(queueLength.current - 1, 0));
      autoplay.current = next !== value;
      return next;
    });
  }, []);

  const previous = useCallback(() => {
    localMutation.current = true;
    setIndex((value) => {
      const previous = Math.max(value - 1, 0);
      autoplay.current = previous !== value;
      return previous;
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
    if (!mediaSession) return;
    mediaSession.setActionHandler("play", toggle);
    mediaSession.setActionHandler("pause", toggle);
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
  }, [next, previous, seek, toggle]);

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
  return (
    <footer className="player">
      <Artwork
        artworkId={player.current.artwork_hash}
        title={player.current.title}
        className="player-cover"
      />
      <div className="player-track">
        <strong>{player.current.title}</strong>
        <span>{player.current.artist ?? t("common.unknownArtist")}</span>
        {player.error ? <small role="alert">{t("player.error")}</small> : null}
      </div>
      <div className="player-controls">
        <button
          type="button"
          onClick={player.previous}
          aria-label={t("player.previous")}
        >
          <Icon name="previous" />
        </button>
        <button
          type="button"
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
    </footer>
  );
}
