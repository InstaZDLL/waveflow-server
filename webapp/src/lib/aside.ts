import type { PodcastEpisode, RadioStation } from "../api";

/**
 * What the player can play beside the queue: a radio station or a podcast
 * episode.
 *
 * Neither is a track. A station has no duration, no next and nothing to
 * scrobble; an episode has a position, but it is kept in a bookmark rather
 * than in the queue the server holds, and the queue holds tracks only. So they
 * play from an element of their own, and the queue is paused under them rather
 * than replaced — playing a track again picks it up where it stopped.
 *
 * That element is never routed through the ReplayGain graph (RFC-022). A
 * station is an arbitrary address on another origin, and a cross-origin
 * stream sent through a `MediaElementSource` without CORS headers plays as
 * silence. An episode is served from this origin and could go through it, but
 * it carries no ReplayGain tags, so there is nothing for the graph to do.
 */
export type Aside =
  | { kind: "live"; station: RadioStation }
  | { kind: "episode"; episode: PodcastEpisode };

/** How often a playing episode's position is written to its bookmark. */
export const SAVE_EVERY_MS = 15_000;

/**
 * What the transport offers for each kind. A station is live: there is no
 * position to show or to seek in. An episode has both. Neither has a next or a
 * previous — an episode that ends stops, and does not roll into the queue —
 * and shuffle and repeat are the queue's.
 */
export function asideTransport(kind: Aside["kind"]): {
  progress: boolean;
  seek: boolean;
} {
  return kind === "live"
    ? { progress: false, seek: false }
    : { progress: true, seek: true };
}

/** What the Media Session shows for an aside. */
export function asideMetadata(
  aside: Aside,
  labels: { live: string; podcasts: string },
): { title: string; artist: string; album: string } {
  return aside.kind === "live"
    ? { title: aside.station.name, artist: labels.live, album: "WaveFlow" }
    : {
        title: aside.episode.title,
        artist: aside.episode.channel_title,
        album: labels.podcasts,
      };
}

/**
 * The cover an aside shows: the station's image, or the episode's channel's
 * cover — both fetched by the server into the artwork store, so this is a
 * hash for `/api/v2/artwork`, never another host's address. `null` keeps the
 * generic symbol.
 */
export function asideArtwork(aside: Aside): string | null {
  return aside.kind === "live"
    ? aside.station.artwork_hash
    : aside.episode.artwork_hash;
}

/**
 * The position an episode resumes from: its bookmark, unless it was left
 * within the last few seconds, which is an episode heard to the end.
 */
export function resumeSeconds(episode: {
  position_ms: number | null;
  duration_ms: number | null;
}): number {
  const position = episode.position_ms ?? 0;
  if (position <= 0) return 0;
  if (episode.duration_ms && episode.duration_ms - position < 5_000) return 0;
  return position / 1000;
}

/**
 * What an episode's bookmark should become: forgotten once the episode has
 * been heard to its end, so that the next play starts it over, and otherwise
 * the place it stopped at.
 */
export function bookmarkWrite(
  currentTime: number,
  ended: boolean,
): { kind: "delete" } | { kind: "set"; positionMs: number } {
  if (ended) return { kind: "delete" };
  return {
    kind: "set",
    positionMs: Math.max(0, Math.round(currentTime * 1000)),
  };
}

/** Whether a playing episode's place is due to be written again. */
export function dueForSave(now: number, lastSaved: number): boolean {
  return now - lastSaved >= SAVE_EVERY_MS;
}

/**
 * Whether a failed play attempt is still worth telling the listener about.
 *
 * Two ways it is not, both reachable by ordinary use. **A stale attempt**:
 * starting a second station replaces the `src` of the one element every
 * station plays from, which rejects the first station's pending `play()` —
 * and that rejection lands after the second station is already on screen, so
 * reporting it would blame a station that is playing perfectly well. **An
 * interruption**: pressing stop while a station is still loading rejects the
 * same promise with `AbortError`, which is the player doing what it was asked
 * rather than a station that cannot be reached.
 */
export function reportsPlaybackFailure(
  attempt: number,
  current: number,
  failure?: unknown,
): boolean {
  if (attempt !== current) return false;
  return !(failure instanceof DOMException && failure.name === "AbortError");
}

/**
 * Whether the aside's element is really sounding.
 *
 * Not `!paused` alone: a source that fails leaves `paused` at false — the
 * failure steps reject the pending play, they do not pause — so a failed
 * station would be taken for one that plays, and every control that asks
 * "is it playing?" would answer backwards: Listen doing nothing, play
 * stopping it, the system's Play ignored and its Pause starting it again.
 */
export function sounding(element: {
  readonly paused: boolean;
  readonly error: unknown;
}): boolean {
  return !element.paused && !element.error;
}

/**
 * The duration an element reports, as the transport shows it. A live stream
 * reports `Infinity`, and an element that has loaded nothing `NaN`; both are
 * "no duration" here.
 */
export function shownDuration(duration: number): number {
  return Number.isFinite(duration) && duration > 0 ? duration : 0;
}
