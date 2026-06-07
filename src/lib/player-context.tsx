// PlayerContext — global state machine for the persistent player.
//
// Design intent:
// - The PlayerBar lives at the document root, not under a route, so
//   playback survives navigation. The context is the single source
//   of truth for "what's playing, what's next, what's been played".
// - The `<audio>` element is OWNED by PlayerBar. The context never
//   touches the DOM — it just exposes the state PlayerBar reads
//   and the actions PlayerBar's UI fires. The element-level event
//   wiring (timeupdate, ended, pause) calls back into the context
//   via the same actions a UI click would.
// - Signed stream URLs expire (HMAC TTL on `waveflow-server`),
//   so we MINT on demand: the queue stores raw `QueueEntry` rows
//   (profileId + libraryId + trackId + display fields), and the
//   `current` slot carries an already-resolved `PlayingTrack` with
//   the URL. `next()` mints lazily — a queue of 200 tracks doesn't
//   trigger 200 server-fn calls on play.
//
// What the context DOESN'T do (yet):
// - Repeat / shuffle modes — coming in Sprint 4.b.
// - Queue reordering UI — same.
// - Cross-fade / gapless — desktop-only concerns; web playback is
//   element-level by design (one `<audio>` at a time).

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react'

import { getStreamUrl } from '@/server-fns/stream'

/**
 * Raw queue / history entry — enough metadata to (a) render the
 * row in a queue panel and (b) mint a signed URL on demand.
 * Deliberately NOT widened to the full `Track` so an editor screen
 * mutating a Track row doesn't have to pipe every column through
 * the player state.
 */
export interface QueueEntry {
  profileId: number
  libraryId: number
  trackId: number
  title: string
  artist?: string
  durationMs: number
}

/**
 * The slot occupied by the live playback. Same shape as
 * `QueueEntry` plus the resolved signed URL the `<audio>` element
 * streams from. Re-mints whenever a queue item is promoted to
 * "current" so the URL is always fresh against the server's TTL.
 */
export interface PlayingTrack extends QueueEntry {
  url: string
}

interface PlayerState {
  current: PlayingTrack | null
  queue: QueueEntry[]
  history: QueueEntry[]
  isPlaying: boolean
  /** Current playback position in seconds (0 when idle). */
  position: number
  /** 0.0 – 1.0. Persisted to localStorage between sessions. */
  volume: number
  /** True while a stream URL is being minted — render a spinner on the play button. */
  isLoading: boolean
}

interface PlayerActions {
  /**
   * Start playback. `contextQueue` is the list of subsequent
   * entries the caller wants to use as auto-advance fuel — pass
   * the slice of the surrounding tracks list AFTER the clicked
   * one. Pass `[]` (or omit) to play a one-off track.
   */
  playTrack: (entry: QueueEntry, contextQueue?: QueueEntry[]) => Promise<void>
  /** Toggle play/pause on the current track. No-op when idle. */
  togglePlayPause: () => void
  /** Advance to the next entry in the queue. No-op when the queue is empty. */
  next: () => Promise<void>
  /**
   * Go back one track. Within the first 3 seconds of the current
   * track, this restarts the current track instead — convention
   * borrowed from every other music app.
   */
  previous: () => Promise<void>
  /** Set the playback position in seconds. */
  seek: (seconds: number) => void
  /** 0.0 – 1.0. Persists to localStorage. */
  setVolume: (volume: number) => void
}

/** Internal: also exposes setters PlayerBar needs to sync from `<audio>` events. */
interface PlayerInternals {
  /** PlayerBar mirrors the `<audio>` element's playing/paused state via this. */
  setIsPlaying: (playing: boolean) => void
  /** PlayerBar mirrors `timeupdate` -> context position. */
  setPosition: (seconds: number) => void
  /**
   * Monotonic counter bumped by each `seek()` call. PlayerBar's
   * effect on this value applies `seekTargetSec` to the
   * `<audio>` element's `currentTime`. Separated from `position`
   * because `position` ticks every 250ms from `onTimeUpdate` —
   * an effect on it would re-fire continuously. The counter
   * isolates the "user asked to seek" signal from the "audio
   * progressed" signal.
   */
  seekVersion: number
  /** The seconds value the latest seek() asked for. */
  seekTargetSec: number
}

export type PlayerContextValue = PlayerState & PlayerActions & PlayerInternals

const PlayerContext = createContext<PlayerContextValue | null>(null)

const VOLUME_STORAGE_KEY = 'waveflow-player-volume'
const DEFAULT_VOLUME = 0.8
/** The "restart vs go-back" threshold in seconds. */
const PREVIOUS_RESTART_THRESHOLD_SEC = 3

function readStoredVolume(): number {
  if (typeof window === 'undefined') return DEFAULT_VOLUME
  const raw = window.localStorage.getItem(VOLUME_STORAGE_KEY)
  if (raw === null) return DEFAULT_VOLUME
  const parsed = Number.parseFloat(raw)
  if (!Number.isFinite(parsed) || parsed < 0 || parsed > 1) return DEFAULT_VOLUME
  return parsed
}

interface PlayerProviderProps {
  children: ReactNode
}

export function PlayerProvider({ children }: PlayerProviderProps) {
  const [current, setCurrent] = useState<PlayingTrack | null>(null)
  const [queue, setQueue] = useState<QueueEntry[]>([])
  const [history, setHistory] = useState<QueueEntry[]>([])
  const [isPlaying, setIsPlaying] = useState(false)
  const [position, setPosition] = useState(0)
  const [volume, setVolumeState] = useState<number>(() => readStoredVolume())
  const [isLoading, setIsLoading] = useState(false)
  const [seekVersion, setSeekVersion] = useState(0)
  const [seekTargetSec, setSeekTargetSec] = useState(0)
  // Monotonic counter that disambiguates concurrent mint requests —
  // a fast next() click while the prior next() is mid-mint would
  // otherwise race, the older URL overwriting the newer track.
  const mintSeqRef = useRef(0)
  // Position drifts as the audio element plays; `previous()` needs
  // to read the latest value without re-rendering through state.
  // Sync via effect because react-hooks/refs (React 19) flags a
  // direct write at render time.
  const positionRef = useRef(0)
  useEffect(() => {
    positionRef.current = position
  }, [position])

  async function mintUrl(entry: QueueEntry): Promise<string> {
    const { url } = await getStreamUrl({
      data: {
        profileId: entry.profileId,
        libraryId: entry.libraryId,
        trackId: entry.trackId,
      },
    })
    return url
  }

  const playTrack = useCallback(
    async (entry: QueueEntry, contextQueue: QueueEntry[] = []): Promise<void> => {
      const seq = ++mintSeqRef.current
      setIsLoading(true)
      try {
        const url = await mintUrl(entry)
        if (seq !== mintSeqRef.current) return
        setCurrent({ ...entry, url })
        setQueue(contextQueue)
        // Reset history so a fresh play() doesn't let `previous()`
        // step into a previous play session's leftovers.
        setHistory([])
        setPosition(0)
        // Autoplay — match the muscle memory every mainstream
        // music player ships. PlayerBar's play/pause effect picks
        // this up and calls `audio.play()` once the element has
        // mounted with the new URL. Only flips when the URL
        // resolved (success branch); a superseded mint never
        // reaches here, an error path falls through to the catch.
        setIsPlaying(true)
      } catch (err) {
        if (seq !== mintSeqRef.current) return
        // Leave `current` / `queue` / `history` / `position`
        // untouched on failure so the UI sits on the prior state
        // rather than going blank.
        console.error('[player] playTrack failed:', err)
        throw err
      } finally {
        if (seq === mintSeqRef.current) setIsLoading(false)
      }
    },
    [],
  )

  const togglePlayPause = useCallback(() => {
    if (!current) return
    setIsPlaying((p) => !p)
  }, [current])

  const next = useCallback(async (): Promise<void> => {
    if (queue.length === 0) return
    const [head, ...rest] = queue
    const seq = ++mintSeqRef.current
    setIsLoading(true)
    try {
      const url = await mintUrl(head)
      if (seq !== mintSeqRef.current) return
      // Push the OLD current to history so `previous()` can find
      // it. Strip the URL — history entries re-mint on revisit.
      setHistory((h) => {
        if (!current) return h
        const { url: _strip, ...raw } = current
        return [raw, ...h]
      })
      setCurrent({ ...head, url })
      setQueue(rest)
      setPosition(0)
      setIsPlaying(true)
    } catch (err) {
      if (seq !== mintSeqRef.current) return
      console.error('[player] next failed:', err)
      throw err
    } finally {
      if (seq === mintSeqRef.current) setIsLoading(false)
    }
  }, [current, queue])

  const previous = useCallback(async (): Promise<void> => {
    // Within the first few seconds, restart the current track
    // rather than going back. Matches every mainstream player.
    if (positionRef.current > PREVIOUS_RESTART_THRESHOLD_SEC) {
      setSeekTargetSec(0)
      setSeekVersion((v) => v + 1)
      setPosition(0)
      return
    }
    if (history.length === 0 || !current) return
    const [head, ...rest] = history
    const seq = ++mintSeqRef.current
    setIsLoading(true)
    try {
      const url = await mintUrl(head)
      if (seq !== mintSeqRef.current) return
      // Old current goes to the FRONT of the queue so a subsequent
      // `next()` walks back forward through the timeline.
      const { url: _strip, ...rawCurrent } = current
      setQueue((q) => [rawCurrent, ...q])
      setCurrent({ ...head, url })
      setHistory(rest)
      setPosition(0)
      setIsPlaying(true)
    } catch (err) {
      if (seq !== mintSeqRef.current) return
      console.error('[player] previous failed:', err)
      throw err
    } finally {
      if (seq === mintSeqRef.current) setIsLoading(false)
    }
  }, [current, history])

  const seek = useCallback((seconds: number) => {
    setSeekTargetSec(seconds)
    setSeekVersion((v) => v + 1)
    setPosition(seconds)
  }, [])

  const setVolume = useCallback((value: number) => {
    const clamped = Math.min(1, Math.max(0, value))
    setVolumeState(clamped)
    if (typeof window !== 'undefined') {
      window.localStorage.setItem(VOLUME_STORAGE_KEY, String(clamped))
    }
  }, [])

  const value = useMemo<PlayerContextValue>(
    () => ({
      current,
      queue,
      history,
      isPlaying,
      position,
      volume,
      isLoading,
      playTrack,
      togglePlayPause,
      next,
      previous,
      seek,
      setVolume,
      setIsPlaying,
      setPosition,
      seekVersion,
      seekTargetSec,
    }),
    [
      current,
      queue,
      history,
      isPlaying,
      position,
      volume,
      isLoading,
      playTrack,
      togglePlayPause,
      next,
      previous,
      seek,
      setVolume,
      seekVersion,
      seekTargetSec,
    ],
  )

  return <PlayerContext.Provider value={value}>{children}</PlayerContext.Provider>
}

export function usePlayer(): PlayerContextValue {
  const ctx = useContext(PlayerContext)
  if (!ctx) {
    throw new Error('usePlayer: must be called inside <PlayerProvider>')
  }
  return ctx
}
