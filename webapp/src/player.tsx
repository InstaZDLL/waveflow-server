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
  formatDuration,
  getQueue,
  type Song,
  saveQueue,
  scrobble,
  streamUrl,
} from "./api";

type PlayerState = {
  queue: Song[];
  index: number;
  current: Song | null;
  playing: boolean;
  position: number;
  duration: number;
  play: (queue: Song[], index: number) => void;
  remove: (index: number) => void;
  clear: () => void;
  toggle: () => void;
  next: () => void;
  previous: () => void;
  seek: (seconds: number) => void;
};

const PlayerContext = createContext<PlayerState | null>(null);

export function usePlayer(): PlayerState {
  const player = useContext(PlayerContext);
  if (!player) throw new Error("usePlayer requires PlayerProvider");
  return player;
}

export function PlayerProvider({ children }: { children: ReactNode }) {
  const audio = useRef<HTMLAudioElement | null>(null);
  const [queue, setQueue] = useState<Song[]>([]);
  const [index, setIndex] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [position, setPosition] = useState(0);
  const [duration, setDuration] = useState(0);
  // A completed listen is reported once per track, when playback passes half
  // of it — the same threshold the Subsonic clients use for a submission.
  const submitted = useRef<string | null>(null);
  // The ended handler is registered once, so it reads the queue length through
  // a ref rather than closing over a stale value.
  const queueLength = useRef(0);
  const queueRef = useRef<Song[]>([]);
  const indexRef = useRef(0);
  const positionRef = useRef(0);
  const hydrated = useRef(false);
  const resumePosition = useRef(0);

  const current = queue[index] ?? null;
  queueLength.current = queue.length;
  queueRef.current = queue;
  indexRef.current = index;
  positionRef.current = position;

  useEffect(() => {
    let cancelled = false;
    void getQueue()
      .then((saved) => {
        if (cancelled || !saved) return;
        setQueue(saved.songs);
        const savedIndex = saved.current
          ? saved.songs.findIndex((song) => song.id === saved.current)
          : 0;
        setIndex(Math.max(savedIndex, 0));
        resumePosition.current = Math.max(saved.position_ms, 0) / 1000;
      })
      // A transient queue failure must not become an unhandled browser error;
      // playback can still start a new queue and retry on its first mutation.
      .catch(() => undefined)
      .finally(() => {
        if (!cancelled) hydrated.current = true;
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!audio.current) audio.current = new Audio();
    const element = audio.current;
    const onTime = () => setPosition(element.currentTime);
    const onDuration = () => setDuration(element.duration || 0);
    // Stop on the last track rather than stepping past it: an out-of-range
    // index empties `current` and the player bar vanishes mid-listen.
    const onEnd = () =>
      setIndex((value) =>
        Math.min(value + 1, Math.max(queueLength.current - 1, 0)),
      );
    const onPlay = () => setPlaying(true);
    const onPause = () => {
      setPlaying(false);
      if (!hydrated.current) return;
      const songs = queueRef.current;
      const selected = songs[indexRef.current] ?? null;
      void saveQueue(
        songs,
        selected?.id ?? null,
        Math.round(positionRef.current * 1000),
      ).catch(() => undefined);
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
  }, []);

  // Loading a track needs a round-trip for its ticket, so guard against a
  // stale response overwriting a newer selection.
  useEffect(() => {
    const element = audio.current;
    if (!element || !current) return;
    let cancelled = false;
    submitted.current = null;
    void (async () => {
      try {
        const url = await streamUrl(current.id);
        if (cancelled) return;
        element.src = url;
        if (resumePosition.current > 0) {
          element.currentTime = resumePosition.current;
          resumePosition.current = 0;
        }
        await element.play();
        void scrobble(current.id, false).catch(() => undefined);
      } catch {
        if (!cancelled) setPlaying(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [current]);

  useEffect(() => {
    if (!current || duration <= 0) return;
    if (submitted.current === current.id) return;
    if (position > duration / 2) {
      submitted.current = current.id;
      void scrobble(current.id, true).catch(() => undefined);
    }
  }, [position, duration, current]);

  const play = useCallback((next: Song[], at: number) => {
    setQueue(next);
    setIndex(at);
  }, []);

  useEffect(() => {
    if (!hydrated.current) return;
    const timeout = window.setTimeout(() => {
      void saveQueue(
        queue,
        current?.id ?? null,
        Math.round(positionRef.current * 1000),
      ).catch(() => undefined);
    }, 400);
    return () => window.clearTimeout(timeout);
  }, [queue, current]);

  const toggle = useCallback(() => {
    const element = audio.current;
    if (!element || !current) return;
    if (element.paused) void element.play();
    else element.pause();
  }, [current]);

  const value = useMemo<PlayerState>(
    () => ({
      queue,
      index,
      current,
      playing,
      position,
      duration,
      play,
      remove: (at: number) => {
        setQueue((songs) => songs.filter((_, position) => position !== at));
        setIndex((value) =>
          value > at
            ? value - 1
            : Math.min(value, Math.max(queue.length - 2, 0)),
        );
      },
      clear: () => {
        audio.current?.pause();
        setQueue([]);
        setIndex(0);
      },
      toggle,
      next: () =>
        setIndex((value) => Math.min(value + 1, Math.max(queue.length - 1, 0))),
      previous: () => setIndex((value) => Math.max(value - 1, 0)),
      seek: (seconds: number) => {
        if (audio.current) audio.current.currentTime = seconds;
      },
    }),
    [queue, index, current, playing, position, duration, play, toggle],
  );

  return (
    <PlayerContext.Provider value={value}>{children}</PlayerContext.Provider>
  );
}

export function PlayerBar() {
  const player = usePlayer();
  const [scrubbing, setScrubbing] = useState<number | null>(null);
  if (!player.current) return null;
  const commit = (value: number) => {
    player.seek(value);
    setScrubbing(null);
  };
  return (
    <footer className="player">
      <div className="player-track">
        <strong>{player.current.title}</strong>
        <span>{player.current.artist ?? "Unknown artist"}</span>
      </div>
      <div className="player-controls">
        <button
          type="button"
          onClick={player.previous}
          aria-label="Previous track"
        >
          Previous
        </button>
        <button
          type="button"
          onClick={player.toggle}
          aria-label={player.playing ? "Pause" : "Play"}
        >
          {player.playing ? "Pause" : "Play"}
        </button>
        <button type="button" onClick={player.next} aria-label="Next track">
          Next
        </button>
      </div>
      <div className="player-progress">
        <span>{formatDuration((scrubbing ?? player.position) * 1000)}</span>
        <input
          type="range"
          min={0}
          max={player.duration || 0}
          step={0.5}
          value={scrubbing ?? player.position}
          onChange={(event) => setScrubbing(Number(event.target.value))}
          onMouseUp={(event) => commit(Number(event.currentTarget.value))}
          onTouchEnd={(event) => commit(Number(event.currentTarget.value))}
          onKeyUp={(event) => commit(Number(event.currentTarget.value))}
          aria-label="Seek"
        />
        <span>{formatDuration(player.duration * 1000)}</span>
      </div>
    </footer>
  );
}
