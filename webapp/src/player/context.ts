// What the provider hands down, and the hooks that read it.
import { createContext, useContext } from "react";

import type { PodcastEpisode, RadioStation, Song } from "../api";
import type { Aside } from "../lib/aside";
import type { RepeatMode } from "./order";

export type PlayerState = {
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

export type PlayerProgress = {
  position: number;
  duration: number;
};

export const PlayerContext = createContext<PlayerState | null>(null);
export const PlayerProgressContext = createContext<PlayerProgress | null>(null);
// The aside's clock, apart from the queue's: the playing screen follows the
// track's position (lyrics, the scrubber), and must not follow an episode's.
export const AsideProgressContext = createContext<PlayerProgress | null>(null);

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
