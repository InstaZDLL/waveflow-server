// A track's music video on the playing screen (RFC-029, the web client).

import { useEffect, useRef, useState } from "react";

import { clipUrl, type Song } from "../../../api";
import { usePlayer, usePlayerProgress } from "../../../player/player";
import { useI18n } from "../../../state/i18n";

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
 * Whether the clip of the track on screen is open. Closed again whenever the
 * track changes, so a track that comes back later opens on its cover, not on a
 * clip it was showing before: the view that holds this is not remounted per
 * track.
 */
export function useClipOpen(trackId: string): {
  open: boolean;
  show: () => void;
  hide: () => void;
} {
  const [state, setState] = useState({ trackId, open: false });
  // Adjusted while rendering, as the detail view adjusts its columns: an
  // effect would paint one frame of the old track's clip over the new one.
  const current = state.trackId === trackId ? state : { trackId, open: false };
  if (current !== state) setState(current);
  return {
    open: current.open,
    show: () => setState({ trackId, open: true }),
    hide: () => setState({ trackId, open: false }),
  };
}

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
 */
export function ClipPlayer({
  song,
  onClose,
}: {
  song: Song;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const player = usePlayer();
  const progress = usePlayerProgress();
  const video = useRef<HTMLVideoElement | null>(null);
  const [state, setState] = useState<ClipState>({ kind: "asking" });
  // Read once, when the clip opens: what to give back when it closes.
  const wasPlaying = useRef(player.playing);
  // Where to put the clip back after a renewed ticket, and whether a renewal
  // has already been tried since the clip last played.
  const restoreAt = useRef<number | null>(null);
  const renewed = useRef(false);
  // The clip's last known position, kept apart from the element: a renewal
  // that is refused takes the element away, and closing must still hand the
  // track the place the clip had reached.
  const lastAt = useRef<number | null>(null);
  // The latest of these, for the cleanup and the handlers, which outlive the
  // render that made them.
  const latest = useRef({ player, duration: progress.duration, onClose });
  latest.current = { player, duration: progress.duration, onClose };

  // Pause the track once, as the clip opens.
  useEffect(() => {
    if (latest.current.player.playing) latest.current.player.toggle();
  }, []);

  // Only one of the two sounds at a time: a track started from the bar, the
  // keyboard or the system controls while the clip is open pauses the clip.
  useEffect(() => {
    if (player.playing) video.current?.pause();
  }, [player.playing]);

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

  function close() {
    const at = resumeAt(
      video.current ? video.current.currentTime : lastAt.current,
      latest.current.duration,
    );
    const { player } = latest.current;
    if (at !== null) player.seek(at);
    if (wasPlaying.current && !player.playing) player.toggle();
    latest.current.onClose();
  }

  function renew() {
    if (renewed.current) {
      setState({ kind: "failed" });
      return;
    }
    renewed.current = true;
    restoreAt.current = video.current?.currentTime ?? lastAt.current;
    clipUrl(song.id)
      .then((ticket) =>
        setState(
          ticket ? { kind: "playing", url: ticket.url } : { kind: "gone" },
        ),
      )
      .catch(() => setState({ kind: "failed" }));
  }

  return (
    <div className="clip-stage">
      {state.kind === "playing" ? (
        // biome-ignore lint/a11y/useMediaCaption: the file is the operator's, played as it is; the server has no caption track to offer
        <video
          ref={video}
          key={state.url}
          className="clip-video"
          src={state.url}
          aria-label={t("playing.clipLabel")}
          controls
          autoPlay
          playsInline
          onLoadedMetadata={(event) => {
            if (restoreAt.current !== null) {
              event.currentTarget.currentTime = restoreAt.current;
              restoreAt.current = null;
            }
          }}
          onTimeUpdate={(event) => {
            lastAt.current = event.currentTarget.currentTime;
          }}
          // The other half of one sound at a time: the clip starting — on
          // opening, or from its own controls after the bar had taken over —
          // pauses the track.
          onPlay={() => {
            const { player } = latest.current;
            if (player.playing) player.toggle();
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
