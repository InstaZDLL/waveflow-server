import { type ChangeEvent, useState } from "react";

import {
  exportPlaylist,
  importPlaylist,
  type PlaylistImport,
  type UnresolvedEntry,
} from "./api";
import { useI18n } from "./i18n";
import { useLibraryScope } from "./library-scope";

/**
 * Arriving with an existing collection's playlists, and leaving with them.
 *
 * `.m3u` is how every other player hands a playlist over, and nothing in this
 * repository read one — which makes it a barrier to adoption rather than a
 * convenience. The server does the reading; this panel exists for the part a
 * server cannot do, which is **asking first**.
 *
 * **Two steps, because a quiet loss is the failure mode.** A playlist file
 * names paths from someone else's filesystem, so some of its entries will not
 * be in this catalogue. An import that silently produced a shorter playlist
 * would leave the listener with no way to know a third of their collection
 * stayed behind, so the file is sent for a dry run first: the server resolves
 * everything, creates nothing, and answers what it could not place and why.
 * Only a press on **Import** writes anything.
 */

/** The file name without its extension: the name to offer for the playlist. */
export function nameFromFile(fileName: string): string {
  const cut = fileName.lastIndexOf(".");
  const stem = cut > 0 ? fileName.slice(0, cut) : fileName;
  return stem.trim() || fileName;
}

function UnresolvedList({ entries }: { entries: UnresolvedEntry[] }) {
  const { t } = useI18n();
  const reason = (entry: UnresolvedEntry) => {
    switch (entry.reason) {
      case "ambiguous":
        return t("import.ambiguous");
      case "not-a-path":
        return t("import.notAPath");
      case "duplicate":
        return t("import.duplicate");
      default:
        return t("import.noMatch");
    }
  };
  return (
    <ul className="list import-unresolved">
      {entries.map((entry) => (
        <li key={`${entry.line}-${entry.entry}`}>
          <span className="import-line">
            {t("import.line", { line: entry.line })}
          </span>
          <code>{entry.title ?? entry.entry}</code>
          <span className="muted">{reason(entry)}</span>
        </li>
      ))}
    </ul>
  );
}

export function PlaylistImportPanel({
  onImported,
}: {
  onImported: () => void;
}) {
  const { t } = useI18n();
  const { active } = useLibraryScope();
  const [preview, setPreview] = useState<{
    file: File;
    report: PlaylistImport;
  } | null>(null);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);

  async function pick(event: ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    // Cleared so that picking the same file again asks again: a change event
    // only fires on a change, and re-choosing a file the user has just fixed
    // is the obvious second thing to do.
    event.target.value = "";
    if (!file) return;
    setBusy(true);
    setFailed(false);
    try {
      const chosen = nameFromFile(file.name);
      const report = await importPlaylist(file, {
        name: chosen,
        libraryId: active?.id,
        dryRun: true,
      });
      setName(chosen);
      setPreview({ file, report });
    } catch {
      setFailed(true);
      setPreview(null);
    } finally {
      setBusy(false);
    }
  }

  async function confirm() {
    if (!preview || busy) return;
    setBusy(true);
    setFailed(false);
    try {
      await importPlaylist(preview.file, {
        name,
        libraryId: active?.id,
        dryRun: false,
      });
      setPreview(null);
      onImported();
    } catch {
      setFailed(true);
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="import-panel">
      <h3>{t("import.title")}</h3>
      <p className="muted">{t("import.detail")}</p>
      <label className="upload-pick">
        <span>{t("import.pick")}</span>
        <input
          type="file"
          accept=".m3u,.m3u8,audio/x-mpegurl,audio/mpegurl"
          onChange={(event) => void pick(event)}
          disabled={busy}
        />
      </label>
      {failed ? <p className="error">{t("import.failed")}</p> : null}
      {preview ? (
        <div className="import-preview" aria-live="polite">
          <label className="inline-form">
            <span>{t("import.name")}</span>
            {/* Labelled by the span beside it and nothing else: an aria-label
                on top of a visible label is a second name for one field, and
                it made the field answer to the same name twice. */}
            <input
              value={name}
              onChange={(event) => setName(event.target.value)}
              required
            />
          </label>
          <p>
            {t("import.found", {
              resolved: preview.report.resolved,
              entries: preview.report.entries,
            })}
          </p>
          {/* Said only when it is worth saying. A file read as Windows-1252
              was not UTF-8, and a wrong guess there mangles accented titles —
              so the reader is told which guess was made rather than left to
              wonder why a name looks odd. */}
          {preview.report.encoding !== "utf-8" ? (
            <p className="muted">
              {t("import.encoding", { encoding: preview.report.encoding })}
            </p>
          ) : null}
          {preview.report.unresolved.length > 0 ? (
            <>
              <p className="muted">
                {t("import.missing", {
                  count: preview.report.unresolved.length,
                })}
              </p>
              <UnresolvedList entries={preview.report.unresolved} />
            </>
          ) : null}
          <div className="actions">
            <button
              type="button"
              onClick={() => void confirm()}
              disabled={busy || !name.trim()}
            >
              {t("import.confirm")}
            </button>
            <button
              type="button"
              onClick={() => setPreview(null)}
              disabled={busy}
            >
              {t("import.cancel")}
            </button>
          </div>
        </div>
      ) : null}
    </section>
  );
}

/**
 * Saves one playlist as a file the next player can read.
 *
 * Fetched rather than linked: the route needs the session's bearer token, and
 * an anchor cannot send one. The object URL is revoked on the way out — it
 * holds the blob alive for as long as the document does otherwise.
 */
export async function downloadPlaylist(id: string, name: string) {
  const file = await exportPlaylist(id);
  const url = URL.createObjectURL(
    new Blob([file], { type: "audio/x-mpegurl" }),
  );
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = `${name || "playlist"}.m3u8`;
  document.body.append(anchor);
  anchor.click();
  anchor.remove();
  // Revoked on a later task, not this one. Browsers differ on when they read
  // the blob behind a download, and revoking in the same task as the click has
  // been reported to cancel it outright. The end-to-end test cannot tell the
  // two apart — it runs Chromium, which starts the download synchronously — so
  // this is a portability fix rather than a bug it caught.
  setTimeout(() => URL.revokeObjectURL(url), 0);
}
