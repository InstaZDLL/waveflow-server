// The immersive view: the artwork and the canvas over the whole screen.

import { useNavigate } from "@tanstack/react-router";
import { useLayoutEffect, useRef } from "react";

import { formatDuration, type Song } from "../../../api";
import { Artwork } from "../../../components/artwork/artwork";
import { CanvasStage } from "../../../components/artwork/canvas";
import { Icon } from "../../../components/layout/icons";
import { usePlayer, usePlayerProgress } from "../../../player/player";
import { useI18n } from "../../../state/i18n";

/**
 * The artwork, and the canvas where there is one, over everything else.
 *
 * A native `<dialog>` opened with `showModal`, which is where the focus trap,
 * the Escape key and the inertness of the page behind come from — the same
 * reasoning the navigation sheet records, and the same reason not to hand-roll
 * an overlay.
 *
 * **It carries transport controls because it covers the player bar.** A view
 * that fills the screen with a cover and leaves no way to pause is a trap
 * rather than a mode. The keyboard shortcuts keep working regardless: the
 * player listens on the window, which a modal dialog does not intercept.
 */
export function ImmersiveView({ song }: { song: Song }) {
  const { t } = useI18n();
  const navigate = useNavigate();
  const player = usePlayer();
  const progress = usePlayerProgress();
  const frame = useRef<HTMLDialogElement | null>(null);
  // Set while this component is going away, so the close that unmounting
  // causes is told apart from the close a person asked for. Without it,
  // leaving the screen by any other route would navigate back onto it.
  const leaving = useRef(false);

  useLayoutEffect(() => {
    const dialog = frame.current;
    if (!dialog) return;
    // **Cleared on the way in, because an effect has to be able to run
    // twice.** `StrictMode` mounts, cleans up and mounts again on the same
    // instance, so the ref survives into the second run — and left at `true`
    // it would swallow a close the person asked for, leaving the view shut and
    // the address still naming it. In development only, which is exactly why
    // no test saw it: the end-to-end suite runs the production build, where
    // the effect runs once.
    leaving.current = false;
    dialog.showModal();
    return () => {
      leaving.current = true;
      if (dialog.open) dialog.close();
    };
  }, []);

  return (
    // **One closer.** The button closes the dialog and nothing else; the
    // navigation hangs off `onClose`, which Escape and the backdrop reach too.
    // A button that both closed and navigated would be a second closer, and a
    // second closer is a second thing to get out of step.
    // biome-ignore lint/a11y/useKeyWithClickEvents: dismissing by keyboard is Escape, which <dialog> handles natively
    <dialog
      ref={frame}
      className="immersive"
      aria-label={t("playing.immersiveTitle")}
      onClose={() => {
        // `close()` queues the event rather than firing it, so the close from
        // that first cleanup can arrive **after** the second `showModal` has
        // reopened the dialog. Navigating then would shut a view the person is
        // looking at, on a close that is already spent.
        if (frame.current?.open) return;
        // **`replace`, or Back reopens what was just closed.** Pushing
        // would leave `/playing → ?view=immersive → /playing` in the
        // history, so the next Back press would land on the immersive entry
        // and open the view again — the opposite of the reason the mode is in
        // the location at all. Replacing swaps the immersive entry for the
        // detail one, and Back goes past both.
        if (!leaving.current)
          void navigate({ to: "/playing", search: {}, replace: true });
      }}
      onClick={(event) => {
        if (event.target === frame.current) frame.current?.close();
      }}
    >
      <CanvasStage trackId={song.id} className="immersive-stage">
        <Artwork
          artworkId={song.artwork_hash}
          title={song.title}
          className="cover immersive-cover"
        />
      </CanvasStage>
      <div className="immersive-detail">
        <span className="eyebrow">{song.album ?? t("common.album")}</span>
        <h2>{song.title}</h2>
        <p className="muted">{song.artist ?? t("common.unknownArtist")}</p>
        <p className="muted">
          <small>
            {formatDuration(progress.position * 1000)}
            {" / "}
            {formatDuration(progress.duration * 1000)}
          </small>
        </p>
        <div className="immersive-transport">
          <button
            type="button"
            aria-label={t("player.previous")}
            onClick={() => player.previous()}
          >
            <Icon name="previous" />
          </button>
          <button
            type="button"
            className="primary"
            aria-label={t(player.playing ? "player.pause" : "player.play")}
            onClick={() => player.toggle()}
          >
            <Icon name={player.playing ? "pause" : "play"} />
          </button>
          <button
            type="button"
            aria-label={t("player.next")}
            onClick={() => player.next()}
          >
            <Icon name="next" />
          </button>
        </div>
      </div>
      <button
        type="button"
        className="immersive-close"
        aria-label={t("playing.leaveImmersive")}
        onClick={() => frame.current?.close()}
      >
        <Icon name="close" />
      </button>
    </dialog>
  );
}
