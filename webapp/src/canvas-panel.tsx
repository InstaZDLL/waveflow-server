import { type ChangeEvent, useState } from "react";

import {
  ApiError,
  canvasUrl,
  placeCanvas,
  removeCanvas,
  type Song,
} from "./api";
import { canvasRefusal, usePrefersReducedMotion } from "./canvas";
import { type TranslationKey, useI18n } from "./i18n";
import { mayPlaceCanvas, useLibraryScope } from "./library-scope";
import { useAsync } from "./pages";

/**
 * What the file picker offers. The server reads the bytes and trusts neither
 * this nor the name, so it only spares someone choosing a file that could never
 * be taken.
 */
const CANVAS_ACCEPT = "video/mp4,video/webm,.mp4,.webm";

/**
 * Places, replaces and removes the loop a track carries (RFC-009).
 *
 * Beside the tags and outside their form: a canvas is not a correction. It is
 * sent the moment a file is chosen, and the form's Save has nothing to do with
 * it.
 *
 * The picker is withheld where the operator has not opened the library to
 * loops, and removal is not: the server does not gate taking a canvas away on
 * that flag, so a closed library still lets its loops go. While the list of
 * libraries is unknown both are offered — the rule the tag editor follows, so a
 * list that failed to load does not turn an owner away, and the server decides.
 */
export function CanvasPanel({ song }: { song: Song }) {
  const { t } = useI18n();
  const reduced = usePrefersReducedMotion();
  const { libraries } = useLibraryScope();
  const library = libraries.find(
    (candidate) => candidate.id === song.library_id,
  );
  const [revision, setRevision] = useState(0);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{
    kind: "notice" | "error";
    text: string;
  } | null>(null);
  // Wrapped, because `useAsync` answers null while it waits and a track that
  // carries no canvas answers null too.
  const { value: looked, error } = useAsync(
    () => canvasUrl(song.id).then((ticket) => ({ ticket })),
    [song.id, revision],
  );

  if (library?.role === "listener") return null;
  const closed = library !== undefined && !mayPlaceCanvas(library);
  const ticket = looked?.ticket ?? null;

  async function change(action: () => Promise<unknown>, done: TranslationKey) {
    setBusy(true);
    setMessage(null);
    try {
      await action();
      setMessage({ kind: "notice", text: t(done) });
    } catch (cause) {
      setMessage({
        kind: "error",
        text: t(canvasRefusal(cause instanceof ApiError ? cause.status : 0)),
      });
    } finally {
      setBusy(false);
      // Read back whatever happened. A refusal can still mean the link moved:
      // a 404 on removal is also a loop another client already took away.
      setRevision((value) => value + 1);
    }
  }

  function pick(event: ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    // Cleared, so choosing the same file again after a refusal is a change.
    event.target.value = "";
    if (file) void change(() => placeCanvas(song.id, file), "canvas.placed");
  }

  return (
    <section className="canvas-panel" aria-labelledby="canvas-panel-title">
      <h2 id="canvas-panel-title">{t("canvas.title")}</h2>
      <p className="muted">{t("canvas.detail")}</p>
      {error ? (
        <p className="error" role="alert">
          {t("canvas.unreadable")}
        </p>
      ) : !looked ? (
        <p className="muted" role="status">
          {t("canvas.loading")}
        </p>
      ) : ticket ? (
        // Controls, unlike the loop on the playing page: here the loop is what
        // is being looked at, and someone who asked for less motion starts it
        // rather than having it start.
        <video
          key={ticket.url}
          className="canvas-preview"
          src={ticket.url}
          aria-label={t("canvas.preview")}
          controls
          loop
          muted
          playsInline
          autoPlay={!reduced}
        />
      ) : (
        <p className="muted">{t("canvas.none")}</p>
      )}
      <div className="canvas-actions">
        {closed ? (
          <p className="muted">{t("canvas.closed")}</p>
        ) : (
          <label className="upload-pick">
            <span>{ticket ? t("canvas.replace") : t("canvas.choose")}</span>
            <input
              type="file"
              accept={CANVAS_ACCEPT}
              disabled={busy}
              onChange={pick}
            />
          </label>
        )}
        {ticket ? (
          <button
            type="button"
            className="danger"
            disabled={busy}
            onClick={() =>
              void change(() => removeCanvas(song.id), "canvas.removed")
            }
          >
            {t("canvas.remove")}
          </button>
        ) : null}
        {busy ? (
          <p className="muted" role="status">
            {t("canvas.saving")}
          </p>
        ) : null}
        {message ? (
          <p
            role={message.kind === "error" ? "alert" : "status"}
            className={message.kind}
          >
            {message.text}
          </p>
        ) : null}
      </div>
    </section>
  );
}
