import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import {
  type CSSProperties,
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
  deleteBookmark,
  formatDuration,
  getQueue,
  type PodcastEpisode,
  type RadioStation,
  type Song,
  saveQueue,
  scrobble,
  setBookmark,
  streamUrl,
} from "../api";
import { podcastChannelQuery } from "../api/queries";
import { Artwork } from "../components/artwork";
import { Icon } from "../components/icons";
import {
  type Aside,
  asideMetadata,
  asideTransport,
  bookmarkWrite,
  dueForSave,
  reportsPlaybackFailure,
  resumeSeconds,
  shownDuration,
  sounding,
} from "../lib/aside";
import { connectWhenRunning, needsWaking } from "../lib/audio-graph";
import { isKeptOffline, keptSongs, offlineTrackUrl } from "../lib/offline";
import { replayGainFactor, routableThroughWebAudio } from "../lib/replay-gain";
import { useI18n } from "../state/i18n";
import { useReplayGainPreference } from "../state/preferences";

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
  /**
   * What plays beside the queue — a station or an episode — or `null`.
   *
   * Everything above is the queue's and keeps meaning the queue: `playing` is
   * whether a track sounds, `toggle` plays or pauses the track. The members
   * below are the aside's. Starting an aside pauses the queue without clearing
   * it; starting a track — `play`, `next`, `previous`, or `toggle` — drops the
   * aside. Only ever one of the two is audible.
   */
  aside: Aside | null;
  asidePlaying: boolean;
  asideError: boolean;
  /** Starts a station, live. Already playing it, does nothing. */
  playLive: (station: RadioStation) => void;
  /**
   * Starts an episode from its bookmark, or carries on with it if it is the
   * one already loaded.
   */
  playEpisode: (episode: PodcastEpisode) => void;
  /** Pauses or resumes the aside. A station stops, and resumes live. */
  toggleAside: () => void;
  /** Seeks in an episode. A station has nowhere to seek to. */
  seekAside: (seconds: number) => void;
  /**
   * Stops the aside and takes it out of the bar, saving an episode's place.
   * Given an id, only if the aside is that station or that episode — read when
   * called, so a caller resuming after a request cannot stop something that
   * started in the meantime.
   */
  stopAside: (only?: string) => void;
};

type PlayerProgress = {
  position: number;
  duration: number;
};

const PlayerContext = createContext<PlayerState | null>(null);
const PlayerProgressContext = createContext<PlayerProgress | null>(null);
// The aside's clock, apart from the queue's: the playing screen follows the
// track's position (lyrics, the scrubber), and must not follow an episode's.
const AsideProgressContext = createContext<PlayerProgress | null>(null);

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

/** Position and duration in seconds of the aside; a station's are zero. */
export function useAsideProgress(): PlayerProgress {
  const progress = useContext(AsideProgressContext);
  if (!progress) throw new Error("useAsideProgress requires PlayerProvider");
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
  const replayGain = useReplayGainPreference();
  const replayGainOn = replayGain.mode !== "off";
  // The Web Audio graph ReplayGain plays through (RFC-022 decision 1): the
  // element, one GainNode, the speakers. Built once and never torn down — an
  // element cannot be detached from its source node — so turning ReplayGain
  // off afterwards sets the gain to one, and a reload restores the plain path.
  const graph = useRef<{ context: AudioContext; gain: GainNode } | null>(null);
  // A context created but not yet running, so the element is not yet claimed.
  // Kept so a later gesture resumes this one rather than making another.
  const candidate = useRef<AudioContext | null>(null);
  const [gainNode, setGainNode] = useState<GainNode | null>(null);
  // Whether the provider is mounted, for a graph whose wiring finishes after
  // a resume: signing out unmounts the player, possibly while one is pending.
  const graphMounted = useRef(false);

  // Signing out unmounts the player, and an open AudioContext holds the audio
  // device until it is closed — browsers cap how many a page may have. Both
  // the candidate and the graph are closed on the way out. The marker is set
  // again on every mount, because StrictMode and Fast Refresh replay effects
  // on an instance that stays mounted.
  useEffect(() => {
    graphMounted.current = true;
    return () => {
      graphMounted.current = false;
      const pending = candidate.current;
      candidate.current = null;
      if (pending) void pending.close().catch(() => undefined);
      const active = graph.current;
      if (active) {
        graph.current = null;
        setGainNode(null);
        void active.context.close().catch(() => undefined);
      }
      // The element is never in the document, so nothing else stops it: with
      // ReplayGain off it would carry on over the sign-in page. With it on,
      // it stays claimed by the context just closed, and would play as
      // silence on an instance that remounts with its refs kept. Either way
      // it is dropped, and the mount effect makes a fresh one.
      const element = audio.current;
      if (element) {
        element.pause();
        element.removeAttribute("src");
        element.load();
        audio.current = null;
      }
      // The next track's preload never plays, but it holds its source and may
      // still be fetching it for an account that has just left.
      const next = preloader.current;
      if (next) {
        next.removeAttribute("src");
        next.load();
        preloader.current = null;
      }
    };
  }, []);

  /**
   * Asks a stopped graph to run again. A play that starts by itself — the
   * next track, repeat-one — is not a gesture, and a browser may refuse; but
   * Safari's `interrupted` state, after a call or a timer, resumes this way.
   */
  const wakeGraph = useCallback(() => {
    const context = graph.current?.context;
    if (context && context.state !== "running") {
      void context.resume().catch(() => undefined);
    }
  }, []);

  /**
   * Builds the graph if ReplayGain wants it, and wakes it if it sleeps.
   *
   * Called from the plays a listener starts, which are always a gesture: a
   * browser creates a context suspended outside one, and refuses to resume it
   * there. Never on page load, so a listener who never asked for ReplayGain
   * keeps the element exactly as it was. The element is claimed only once the
   * context runs (`connectWhenRunning`): claimed by a suspended context, it
   * would play as silence.
   */
  const prepareGraph = useCallback(() => {
    const element = audio.current;
    if (!element) return;
    if (graph.current) {
      wakeGraph();
      return;
    }
    if (!replayGainOn) return;
    if (candidate.current) {
      // Still waiting to run: this gesture may be the one it needed.
      void candidate.current.resume().catch(() => undefined);
      return;
    }
    // A cross-origin source would play as silence through the graph. The
    // tickets are relative, so this holds by construction; checked anyway.
    const source = element.currentSrc || element.src;
    if (source && !routableThroughWebAudio(source, window.location.href))
      return;
    const Context =
      window.AudioContext ??
      (window as { webkitAudioContext?: typeof AudioContext })
        .webkitAudioContext;
    if (!Context) return;
    let context: AudioContext;
    try {
      context = new Context();
    } catch {
      return;
    }
    candidate.current = context;
    void connectWhenRunning(context, () => {
      const gain = context.createGain();
      // Last, because it is the step that cannot be undone.
      context
        .createMediaElementSource(element)
        .connect(gain)
        .connect(context.destination);
      return gain;
    }).then((gain) => {
      if (candidate.current === context) candidate.current = null;
      if (!graphMounted.current) {
        // Unmounted while it waited: nobody will ever close it otherwise.
        void context.close().catch(() => undefined);
        return;
      }
      // No graph: the element keeps playing as it did, unlevelled.
      if (!gain) return;
      graph.current = { context, gain };
      setGainNode(gain);
      // Stopped under a playing element — Safari interrupts a context for a
      // call or a timer — it is asked to run again rather than left silent.
      context.addEventListener("statechange", () => {
        if (needsWaking(context.state, audio.current?.paused ?? true))
          wakeGraph();
      });
    });
  }, [replayGainOn, wakeGraph]);

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

  // ── The aside: a station or an episode, beside the queue ──────────────
  //
  // Its own element, and deliberately so. The queue's element is the one the
  // ReplayGain graph claims, and once claimed its sound reaches the speakers
  // only through the context: a station on another origin without CORS
  // headers would play there as silence. Nothing below ever hands this one to
  // `createMediaElementSource`, so a station plays as any `<audio>` does.
  const queryClient = useQueryClient();
  const side = useRef<HTMLAudioElement | null>(null);
  const [aside, setAside] = useState<Aside | null>(null);
  const asideRef = useRef<Aside | null>(null);
  const [asidePlaying, setAsidePlaying] = useState(false);
  const [asideError, setAsideError] = useState(false);
  const [asidePosition, setAsidePosition] = useState(0);
  const [asideDuration, setAsideDuration] = useState(0);
  const asidePositionRef = useRef(0);
  // Which play attempt the aside is showing, as the radio page counted them:
  // one element serves every station and episode, so starting another rejects
  // the pending `play()` of the last, after the new one is on screen.
  const sideAttempt = useRef(0);
  const lastBookmark = useRef(0);
  const bookmarkChain = useRef<Promise<void>>(Promise.resolve());

  /**
   * Writes the place of the episode playing aside to its bookmark — the same
   * route the podcasts page used, under the episode's id. `refresh` asks the
   * page to read the episode again, so the place it shows is the one kept.
   */
  const saveEpisode = useCallback(
    (refresh: boolean) => {
      const current = asideRef.current;
      const element = side.current;
      if (current?.kind !== "episode" || !element) return;
      // No source yet — its ticket is on its way — or no longer: there is no
      // place to keep. Once it has one, `currentTime` is the place even before
      // the metadata, because the resume was set as its start position.
      if (!element.getAttribute("src")) return;
      const { id, channel_id: channel } = current.episode;
      const write = bookmarkWrite(element.currentTime, element.ended);
      lastBookmark.current = Date.now();
      // One after the other, each sent once the last has answered. Two
      // requests in flight together may be served in either order: a seek's
      // `PUT` still on its way when the episode ends would land after the
      // `DELETE` and bring back the bookmark of an episode heard to the end.
      bookmarkChain.current = bookmarkChain.current
        .then(() =>
          write.kind === "delete"
            ? deleteBookmark(id)
            : setBookmark(id, write.positionMs),
        )
        .then(() => {
          if (refresh)
            void queryClient.invalidateQueries({
              queryKey: podcastChannelQuery(channel).queryKey,
            });
        })
        // A place that did not save is the previous one; the next save
        // writes what this one would have.
        .catch(() => undefined);
    },
    [queryClient],
  );

  /** Silences the aside's element and lets go of its source. */
  const silenceSide = useCallback(() => {
    // Moved before the pause, because the pause is what rejects the pending
    // attempt, and that rejection must not report a failure.
    sideAttempt.current += 1;
    // Said here rather than left to the `pause` event: `load()` below drops
    // the events the element still had queued, that one among them, and the
    // bar would go on offering to stop a station that has stopped.
    setAsidePlaying(false);
    const element = side.current;
    if (!element) return;
    element.pause();
    element.removeAttribute("src");
    element.load();
  }, []);

  const stopAside = useCallback(
    (only?: string) => {
      const current = asideRef.current;
      if (!current) return;
      const id =
        current.kind === "live" ? current.station.id : current.episode.id;
      if (only !== undefined && only !== id) return;
      saveEpisode(true);
      silenceSide();
      asideRef.current = null;
      setAside(null);
      setAsidePlaying(false);
      setAsideError(false);
      asidePositionRef.current = 0;
      setAsidePosition(0);
      setAsideDuration(0);
    },
    [saveEpisode, silenceSide],
  );

  /** The queue goes quiet under an aside; it is paused, never cleared. */
  const pauseQueue = useCallback(() => {
    const element = audio.current;
    if (element && !element.paused) element.pause();
  }, []);

  /** Puts an aside in the bar, before its first sample. */
  const showAside = useCallback((next: Aside, at: number, length: number) => {
    asideRef.current = next;
    setAside(next);
    setAsideError(false);
    asidePositionRef.current = at;
    setAsidePosition(at);
    setAsideDuration(length);
  }, []);

  useEffect(() => {
    const element = new Audio();
    element.preload = "auto";
    side.current = element;
    const onPlay = () => {
      setAsidePlaying(true);
      // One audible source. Started from anywhere — the bar, a page, the
      // Media Session — the aside silences the queue.
      const queue = audio.current;
      if (queue && !queue.paused) queue.pause();
    };
    // Ending is a pause too, fired with `ended` set, so this one write also
    // forgets the bookmark of an episode heard to the end.
    const onPause = () => {
      setAsidePlaying(false);
      saveEpisode(true);
    };
    const onTime = () => {
      asidePositionRef.current = element.currentTime;
      setAsidePosition(element.currentTime);
      if (!element.paused && dueForSave(Date.now(), lastBookmark.current))
        saveEpisode(false);
    };
    const onDuration = () => setAsideDuration(shownDuration(element.duration));
    const onError = () => {
      // Letting go of a source is not a failure of it.
      if (!element.getAttribute("src")) return;
      setAsidePlaying(false);
      setAsideError(true);
    };
    element.addEventListener("play", onPlay);
    element.addEventListener("pause", onPause);
    element.addEventListener("timeupdate", onTime);
    element.addEventListener("loadedmetadata", onDuration);
    element.addEventListener("durationchange", onDuration);
    element.addEventListener("error", onError);
    return () => {
      element.removeEventListener("play", onPlay);
      element.removeEventListener("pause", onPause);
      element.removeEventListener("timeupdate", onTime);
      element.removeEventListener("loadedmetadata", onDuration);
      element.removeEventListener("durationchange", onDuration);
      element.removeEventListener("error", onError);
      // Signing out unmounts the player, and a station or an episode stops
      // with it, as the music does (#329, #330). The listeners go first: the
      // session that could write a bookmark is already gone, and the place
      // kept is the last periodic save.
      //
      // The attempt moves first. An episode whose ticket is still on its way
      // would otherwise pass its own check when the ticket lands, and set its
      // source and play on this element — after the sign-out, with no bar
      // left to stop it.
      sideAttempt.current += 1;
      element.pause();
      element.removeAttribute("src");
      element.load();
      if (side.current === element) side.current = null;
      asideRef.current = null;
    };
  }, [saveEpisode]);

  const playLive = useCallback(
    (station: RadioStation) => {
      const element = side.current;
      if (!element) return;
      const previous = asideRef.current;
      // Already playing it, unless it failed: a source that fails leaves
      // `paused` false, and the station must be tried again rather than taken
      // for one that plays.
      if (
        previous?.kind === "live" &&
        previous.station.id === station.id &&
        sounding(element)
      )
        return;
      if (previous?.kind === "episode") saveEpisode(true);
      pauseQueue();
      sideAttempt.current += 1;
      const mine = sideAttempt.current;
      showAside({ kind: "live", station }, 0, 0);
      // Set afresh on every start, including a resume: a station stopped and
      // started again joins the broadcast where it is now, not where the
      // browser stopped buffering it.
      element.src = station.stream_url;
      void element.play().catch((failure: unknown) => {
        if (!reportsPlaybackFailure(mine, sideAttempt.current, failure)) return;
        setAsidePlaying(false);
        setAsideError(true);
      });
    },
    [pauseQueue, saveEpisode, showAside],
  );

  const playEpisode = useCallback(
    (episode: PodcastEpisode) => {
      const element = side.current;
      if (!element) return;
      const previous = asideRef.current;
      if (
        previous?.kind === "episode" &&
        previous.episode.id === episode.id &&
        element.getAttribute("src") &&
        !element.error
      ) {
        // Already loaded: carry on from where it is, and not from the
        // bookmark the page read before it was last paused.
        if (element.paused) {
          pauseQueue();
          void element.play().catch(() => undefined);
        }
        return;
      }
      if (previous?.kind === "episode") saveEpisode(true);
      pauseQueue();
      // Whatever played aside goes quiet now, not when the ticket arrives.
      silenceSide();
      const mine = sideAttempt.current;
      const resume = resumeSeconds(episode);
      showAside(
        { kind: "episode", episode },
        resume,
        episode.duration_ms ? episode.duration_ms / 1000 : 0,
      );
      lastBookmark.current = Date.now();
      void (async () => {
        try {
          const { url } = await streamUrl(episode.id);
          if (mine !== sideAttempt.current) return;
          element.src = url;
          // Before any metadata, this sets where playback will start, and is
          // what `currentTime` answers until then: a pause in that window
          // keeps the bookmark it resumed from rather than writing zero.
          if (resume > 0) element.currentTime = resume;
          await element.play();
        } catch (failure) {
          if (!reportsPlaybackFailure(mine, sideAttempt.current, failure))
            return;
          setAsidePlaying(false);
          setAsideError(true);
        }
      })();
    },
    [pauseQueue, saveEpisode, showAside, silenceSide],
  );

  const toggleAside = useCallback(() => {
    const element = side.current;
    const current = asideRef.current;
    if (!element || !current) return;
    // A failed source is not playing, whatever `paused` says (see `playLive`):
    // pressing play on it tries it again.
    if (sounding(element)) {
      if (current.kind === "live") {
        // Stopped rather than paused: a live stream left paused keeps its
        // connection, and resuming it would play the past.
        silenceSide();
      } else {
        element.pause();
      }
      return;
    }
    if (current.kind === "live") {
      playLive(current.station);
      return;
    }
    if (element.error || !element.getAttribute("src")) {
      playEpisode(current.episode);
      return;
    }
    pauseQueue();
    setAsideError(false);
    void element.play().catch(() => undefined);
  }, [pauseQueue, playEpisode, playLive, silenceSide]);

  const seekAside = useCallback(
    (seconds: number) => {
      const element = side.current;
      if (!element || asideRef.current?.kind !== "episode") return;
      const length = shownDuration(element.duration);
      const bounded = Math.max(0, length ? Math.min(seconds, length) : seconds);
      element.currentTime = bounded;
      asidePositionRef.current = bounded;
      setAsidePosition(bounded);
      saveEpisode(false);
    },
    [saveEpisode],
  );

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
  }, [hydrated, persistQueue, stopAside, wakeGraph]);

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
  }, [current, resolveStream, wakeGraph]);

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
  ]);

  useEffect(() => {
    if (!navigator.mediaSession) return;
    if (aside) {
      navigator.mediaSession.playbackState = asidePlaying
        ? "playing"
        : "paused";
      navigator.mediaSession.metadata = new MediaMetadata({
        ...asideMetadata(aside, {
          live: t("player.liveRadio"),
          podcasts: t("nav.podcasts"),
        }),
        artwork: [],
      });
      return;
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
  }, [seek, seekAside, toggle, toggleAside]);

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

/**
 * The line under the transport: where playback is, and a range to move it.
 * Shared by the queue and an episode, each with its own clock.
 */
function Scrubber({
  position,
  duration,
  onSeek,
}: {
  position: number;
  duration: number;
  onSeek: (seconds: number) => void;
}) {
  const { t } = useI18n();
  const [scrubbing, setScrubbing] = useState<number | null>(null);
  const commit = (value: number) => {
    onSeek(value);
    setScrubbing(null);
  };
  return (
    // `--played` is the share already heard. The dark themes draw that part
    // of the line as the mark's wave and leave the rest straight, beneath a
    // range input that keeps every interaction.
    <div
      className="player-progress"
      style={
        {
          "--played":
            duration > 0 ? Math.min(1, (scrubbing ?? position) / duration) : 0,
        } as CSSProperties
      }
    >
      <span>{formatDuration((scrubbing ?? position) * 1000)}</span>
      <span className="progress-wave" aria-hidden="true" />
      <input
        type="range"
        min={0}
        max={duration || 0}
        step={0.5}
        value={scrubbing ?? position}
        onChange={(event) => setScrubbing(Number(event.target.value))}
        onMouseUp={(event) => commit(Number(event.currentTarget.value))}
        onTouchEnd={(event) => commit(Number(event.currentTarget.value))}
        onKeyUp={(event) => commit(Number(event.currentTarget.value))}
        aria-label={t("player.seek")}
      />
      <span>{formatDuration(duration * 1000)}</span>
    </div>
  );
}

function QueueScrubber() {
  const player = usePlayer();
  const progress = usePlayerProgress();
  return (
    <Scrubber
      position={progress.position}
      duration={progress.duration}
      onSeek={player.seek}
    />
  );
}

function AsideScrubber() {
  const player = usePlayer();
  const progress = useAsideProgress();
  return (
    <Scrubber
      position={progress.position}
      duration={progress.duration}
      onSeek={player.seekAside}
    />
  );
}

/** The level, which is the same for the queue and for an aside. */
function LevelControls() {
  const player = usePlayer();
  const { t } = useI18n();
  return (
    <>
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
    </>
  );
}

/**
 * The bar while a station or an episode plays aside from the queue.
 *
 * A station is live: a badge says so, and there is nothing to seek in and no
 * next to go to. An episode has its position and a scrubber, and no next
 * either, because an episode that ends stops. Shuffle and repeat are the
 * queue's and are not offered. The second button takes the aside out of the
 * bar, which leaves the queue's track there, paused where it was.
 */
function AsideBar({ aside }: { aside: Aside }) {
  const player = usePlayer();
  const { t } = useI18n();
  const live = aside.kind === "live";
  const transport = asideTransport(aside.kind);
  const primaryLabel = player.asidePlaying
    ? live
      ? t("player.stopLive")
      : t("player.pause")
    : t("player.play");
  return (
    <footer className="player player-with-aside" data-aside={aside.kind}>
      <span className="cover player-cover cover-fallback" aria-hidden="true">
        <Icon name={live ? "radio" : "podcasts"} size={26} />
      </span>
      <div className="player-track">
        <strong>{live ? aside.station.name : aside.episode.title}</strong>
        {live ? (
          <span className="badge live-badge">{t("player.live")}</span>
        ) : (
          <span>{aside.episode.channel_title}</span>
        )}
        {player.asideError ? (
          <small role="alert">
            {live ? t("player.liveError") : t("podcasts.playbackError")}
          </small>
        ) : null}
      </div>
      <div className="player-controls">
        <button
          type="button"
          className="primary"
          onClick={player.toggleAside}
          aria-label={primaryLabel}
        >
          <Icon
            name={player.asidePlaying ? (live ? "stop" : "pause") : "play"}
            size={24}
          />
        </button>
        <button
          type="button"
          onClick={() => player.stopAside()}
          aria-label={t("player.closeAside")}
        >
          <Icon name="close" />
        </button>
      </div>
      {transport.progress ? <AsideScrubber /> : null}
      <div className="player-aside">
        <LevelControls />
      </div>
    </footer>
  );
}

/** Render the persistent controls for the currently selected track. */
export function PlayerBar() {
  const player = usePlayer();
  const { t } = useI18n();
  if (player.aside) return <AsideBar aside={player.aside} />;
  if (!player.current) return null;
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
      <QueueScrubber />
      <div className="player-aside">
        <LevelControls />
      </div>
    </footer>
  );
}
