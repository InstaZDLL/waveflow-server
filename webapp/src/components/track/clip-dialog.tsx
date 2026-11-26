// A track's music video in a dialog of its own, opened from the track's menu.

import { useLayoutEffect, useRef } from "react";

import type { Song } from "../../api";
import { useI18n } from "../../state/i18n";
import { ClipPlayer } from "./clip";

/**
 * The clip of any track in the library, not only the one playing.
 *
 * A native `<dialog>` opened with `showModal`, as Properties is: the focus
 * trap and the inertness of the page behind come from the platform.
 *
 * **Every way out runs the clip's own close.** Escape and a click on the
 * backdrop would shut the dialog by themselves and take the clip away without
 * giving the track back, paused for good by a clip nobody is watching any
 * more. Both are turned into the press of the clip's button, which resumes the
 * track if it was playing and then closes the dialog through `onClose`.
 */
export function ClipDialog({
  song,
  onClose,
}: {
  song: Song;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const frame = useRef<HTMLDialogElement | null>(null);
  const closer = useRef<(() => void) | null>(null);

  useLayoutEffect(() => {
    const dialog = frame.current;
    if (!dialog) return;
    dialog.showModal();
    return () => {
      if (dialog.open) dialog.close();
    };
  }, []);

  function leave() {
    if (closer.current) closer.current();
    else onClose();
  }

  return (
    // biome-ignore lint/a11y/useKeyWithClickEvents: dismissing by keyboard is Escape, which <dialog> handles natively
    <dialog
      ref={frame}
      className="clip-dialog"
      aria-label={t("playing.clipLabel")}
      onCancel={(event) => {
        event.preventDefault();
        leave();
      }}
      onClick={(event) => {
        if (event.target === frame.current) leave();
      }}
    >
      <header className="clip-dialog-header">
        <strong>{song.title}</strong>
        <span className="muted">
          {song.artist ?? t("common.unknownArtist")}
        </span>
      </header>
      <ClipPlayer
        song={song}
        onClose={onClose}
        closer={closer}
        className="clip-dialog-stage"
      />
    </dialog>
  );
}
