// A track's music video (RFC-029, the web client): on the playing screen, in
// the immersive view, and from a track's menu.

import { useEffect, useRef, useState } from "react";

import { clipUrl, type Song } from "../../api";
import { usePlayer, usePlayerProgress } from "../../player/player";
import { useI18n } from "../../state/i18n";

/**
 * Where the track takes over once its clip closes: at the clip's position,
 * as on the desktop, unless the clip ran past the end of the track — a video
 * often carries an intro or an outro the album version has not — in which
 * case at the track's start, rather than at an end it would leave at once.
 * `null` when the clip never played, so the track stays where it was.
 */
export function resumeAt(
  clipPosition: number | null,
  trackDuration: number,
): number | null {
  if (clipPosition === null || !Number.isFinite(clipPosition)) return null;
  if (clipPosition < 0) return 0;
  return trackDuration > 0 && clipPosition >= trackDuration ? 0 : clipPosition;
}

type ClipState =
  | { kind: "asking" }
  | { kind: "playing"; url: string }
  /** The server says there is no clip any more: gone since the scan. */
  | { kind: "gone" }
  /** The browser could not play the container, or the server did not answer. */
  | { kind: "failed" };

/**
 * What one opening of a clip carries from a player that goes away to the one
 * that takes over: the detail view and the immersive view each show the clip
 * in their own place, so moving from one to the other mounts a new element.
 * Without this, the clip would start over, and closing it would no longer know
 * whether the track was playing when it opened.
 */
export type ClipSession = {
  /** Whether the track was playing when the clip opened; `null` until then. */
  wasPlaying: boolean | null;
  /** Where the clip stood, in seconds; `null` before it played. */
  at: number | null;
  /** Whether the clip was playing when its element went away. */
  playing: boolean;
  /**
   * Gives the track back as closing the clip would, without closing it: set
   * by the player that shows the clip, run by whoever holds the session when
   * it goes away with the clip still open. The player cannot do it on its own
   * unmounting, which is also how the clip moves from one view to the next.
   */
  giveBack?: () => void;
};

const freshSession = (): ClipSession => ({
  wasPlaying: null,
  at: null,
  playing: true,
});

/**
 * Whether the clip of the track on screen is open. Closed again whenever the
 * track changes, so a track that comes back later opens on its cover, not on a
 * clip it was showing before: the view that holds this is not remounted per
 * track.
 *
 * **Leaving with the clip open gives the track back.** The playing screen
 * holds this; a listener who goes to another page with the clip still open
 * takes the clip away with the screen, and the track it paused would stay
 * paused with nothing on screen to say why. A change of track gives nothing
 * back: the queue moved on, and what plays now is the listener's.
 */
export function useClipOpen(trackId: string): {
  open: boolean;
  show: () => void;
  hide: () => void;
  session: ClipSession;
} {
  const [state, setState] = useState(() => ({
    trackId,
    open: false,
    session: freshSession(),
  }));
  // Adjusted while rendering, as the detail view adjusts its columns: an
  // effect would paint one frame of the old track's clip over the new one.
  const current =
    state.trackId === trackId
      ? state
      : { trackId, open: false, session: freshSession() };
  if (current !== state) setState(current);
  // Only an open clip's session has a player to give back through: closing,
  // and a change of track, both start a fresh session that none has claimed.
  const live = useRef(current.session);
  live.current = current.session;
  useEffect(() => () => live.current.giveBack?.(), []);
  return {
    open: current.open,
    session: current.session,
    show: () => setState({ trackId, open: true, session: freshSession() }),
    hide: () => setState({ trackId, open: false, session: freshSession() }),
  };
}

/** An opening of the clip, as {@link useClipOpen} keeps it. */
export type ClipOpen = ReturnType<typeof useClipOpen>;

/**
 * The clip in place of the cover, with its own sound and its own length.
 *
 * The track pauses while the clip plays, and takes over at the clip's position
 * when it closes — by the button, or by the clip reaching its end — and plays
 * on only if it was playing when the clip opened.
 *
 * A ticket lasts an hour, and a clip paused longer than that answers its next
 * range request with a 404, which the element reports as an error. One new
 * ticket is asked for then, and the clip resumes where it stood; only a 404 to
 * that question means the clip is gone.
 *
 * **The clip may be another track's**, opened from a track's menu while
 * something else is in the queue. The current track then pauses and resumes
 * as it does for its own clip, but stays where it was: the clip's position is
 * a place in a different recording.
 *
 * `session` is what a clip carries from one view to the next (see
 * {@link ClipSession}); a player that is the only one for its opening, as in a
 * dialog, needs none. `closer` hands the caller the same close the button
 * runs, for a dialog's Escape key, which would otherwise take the clip away
 * without giving the track back.
 */
export function ClipPlayer({
  song,
  onClose,
  session,
  closer,
  className,
}: {
  song: Song;
  onClose: () => void;
  session?: ClipSession;
  closer?: { current: (() => void) | null };
  className?: string;
}) {
  const { t } = useI18n();
  const player = usePlayer();
  const progress = usePlayerProgress();
  const video = useRef<HTMLVideoElement | null>(null);
  const [state, setState] = useState<ClipState>({ kind: "asking" });
  // Read once, when the clip opens: what to give back when it closes. A player
  // taking over from another reads what the first one read, not the paused
  // track the first one left behind.
  const wasPlaying = useRef<boolean | null>(null);
  if (wasPlaying.current === null) {
    wasPlaying.current = session?.wasPlaying ?? player.playing;
    if (session) session.wasPlaying = wasPlaying.current;
  }
  // And which track that was. The clip's dialog is modal, but the media keys
  // and the system controls are not: a track changed while the clip played is
  // the listener's choice, and closing the clip must not start it.
  const openedOn = useRef(player.current?.id ?? null);
  // Whether this element starts playing on its own: always for a new opening,
  // and for one carried over only if the clip was playing when it left.
  const startPlaying = useRef(session?.playing ?? true);
  // Where to put the clip back after a renewed ticket or a change of view, and
  // whether a renewal has already been tried since the clip last played.
  const restoreAt = useRef<number | null>(session?.at ?? null);
  const renewed = useRef(false);
  // The clip's last known position, kept apart from the element: a renewal
  // that is refused takes the element away, and closing must still hand the
  // track the place the clip had reached.
  const lastAt = useRef<number | null>(session?.at ?? null);
  // The latest of these, for the cleanup and the handlers, which outlive the
  // render that made them.
  const latest = useRef({ player, duration: progress.duration, onClose });
  latest.current = { player, duration: progress.duration, onClose };

  // Pause the track once, as the clip opens — not when a clip that was paused
  // comes back in another view, where the track may have been started since.
  useEffect(() => {
    if (startPlaying.current && latest.current.player.playing) {
      latest.current.player.toggle();
    }
  }, []);

  useEffect(() => {
    if (!closer) return;
    closer.current = close;
    return () => {
      closer.current = null;
    };
  });

  // Whether this opening has given the track back already, and the way to,
  // for the cleanup below, which outlives the render that made it.
  const givenBack = useRef(false);
  const lone = useRef(!session);
  const giveBackLater = useRef(giveBack);
  giveBackLater.current = giveBack;
  useEffect(() => {
    // Cleared on the way in: `StrictMode` runs this twice on one instance.
    givenBack.current = false;
    // A player of its own, as in a dialog, gives the track back when it goes:
    // a dialog unmounted by Back is no less closed than one closed by Escape.
    // A player on a session is unmounted by a change of view too, so its
    // session's holder decides (see `useClipOpen`).
    return () => {
      if (lone.current) giveBackLater.current();
    };
  }, []);
  useEffect(() => {
    if (session) session.giveBack = giveBack;
  });

  // Only one sound at a time: a track started from the bar, the keyboard or the
  // system controls while the clip is open pauses the clip, and so does a
  // station or an episode started beside the queue.
  useEffect(() => {
    if (player.playing || player.asidePlaying) video.current?.pause();
  }, [player.playing, player.asidePlaying]);

  useEffect(() => {
    let cancelled = false;
    clipUrl(song.id)
      .then((ticket) => {
        if (cancelled) return;
        setState(
          ticket ? { kind: "playing", url: ticket.url } : { kind: "gone" },
        );
      })
      .catch(() => {
        if (!cancelled) setState({ kind: "failed" });
      });
    return () => {
      cancelled = true;
    };
  }, [song.id]);

  /**
   * Where the clip stands. A place still waiting for the element's metadata
   * comes first: until then the new element reads 0, and a clip closed or
   * renewed just after a change of view would hand on its start.
   */
  function position(): number | null {
    if (restoreAt.current !== null) return restoreAt.current;
    return video.current ? video.current.currentTime : lastAt.current;
  }

  /**
   * The track back as the clip found it: at the clip's place when it is the
   * clip's own, and playing if it was. Once per opening, whichever way the
   * clip goes — the button, its end, Escape, or the page going away.
   */
  function giveBack() {
    if (givenBack.current) return;
    givenBack.current = true;
    const at = resumeAt(position(), latest.current.duration);
    const { player } = latest.current;
    if (at !== null && player.current?.id === song.id) player.seek(at);
    // Not over a station or an episode started while the clip was open, playing
    // or paused since by the clip itself: `toggle` would drop it from the bar,
    // and closing a clip must not take away what the listener chose since. A
    // track that was playing when the clip opened had no aside beside it, so
    // one here was started meanwhile.
    if (
      wasPlaying.current &&
      !player.playing &&
      !player.aside &&
      player.current?.id === openedOn.current
    ) {
      player.toggle();
    }
  }

  function close() {
    giveBack();
    latest.current.onClose();
  }

  function renew() {
    if (renewed.current) {
      setState({ kind: "failed" });
      return;
    }
    renewed.current = true;
    restoreAt.current = position();
    clipUrl(song.id)
      .then((ticket) =>
        setState(
          ticket ? { kind: "playing", url: ticket.url } : { kind: "gone" },
        ),
      )
      .catch(() => setState({ kind: "failed" }));
  }

  return (
    <div className={className ? `clip-stage ${className}` : "clip-stage"}>
      {state.kind === "playing" ? (
        // biome-ignore lint/a11y/useMediaCaption: the file is the operator's, played as it is; the server has no caption track to offer
        <video
          ref={video}
          key={state.url}
          className="clip-video"
          src={state.url}
          aria-label={t("playing.clipLabel")}
          controls
          autoPlay={startPlaying.current}
          playsInline
          onLoadedMetadata={(event) => {
            if (restoreAt.current !== null) {
              event.currentTarget.currentTime = restoreAt.current;
              restoreAt.current = null;
            }
          }}
          onTimeUpdate={(event) => {
            lastAt.current = event.currentTarget.currentTime;
            if (session) session.at = lastAt.current;
          }}
          onPause={() => {
            if (session) session.playing = false;
          }}
          // The other half of one sound at a time: the clip starting — on
          // opening, or from its own controls after the bar had taken over —
          // pauses the track.
          onPlay={() => {
            // A clip started once plays on by itself after a renewed ticket.
            startPlaying.current = true;
            if (session) session.playing = true;
            const { player } = latest.current;
            if (player.playing) player.toggle();
            // A station or an episode plays while the queue is paused, so
            // the queue's state alone would let it sound over the clip.
            // Paused, not closed: a station resumes live, an episode where
            // it stood.
            if (player.asidePlaying) player.toggleAside();
          }}
          onPlaying={() => {
            renewed.current = false;
          }}
          onEnded={close}
          onError={renew}
        />
      ) : (
        <p className="clip-message muted" role="status">
          {state.kind === "asking"
            ? t("common.loading")
            : state.kind === "gone"
              ? t("playing.clipGone")
              : t("playing.clipError")}
        </p>
      )}
      <button type="button" className="clip-close" onClick={close}>
        {t("playing.backToTrack")}
      </button>
    </div>
  );
}
