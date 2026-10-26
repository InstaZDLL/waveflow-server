// The aside: a station or an episode, on its own element beside the queue.
import { useQueryClient } from "@tanstack/react-query";
import {
  type RefObject,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";

import {
  deleteBookmark,
  type PodcastEpisode,
  type RadioStation,
  setBookmark,
  streamUrl,
} from "../api";
import { podcastChannelQuery } from "../api/queries";
import {
  type Aside,
  bookmarkWrite,
  dueForSave,
  reportsPlaybackFailure,
  resumeSeconds,
  shownDuration,
  sounding,
} from "../lib/aside";

/** Part of `PlayerProvider`, called in its place among the provider's hooks. */
export function useAsidePlayback(audio: RefObject<HTMLAudioElement | null>) {
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
  }, [audio]);

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
  }, [saveEpisode, audio]);

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

  return {
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
  };
}
