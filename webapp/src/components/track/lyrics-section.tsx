import { useQueryClient } from "@tanstack/react-query";
import { useRef, useState } from "react";

import {
  ApiError,
  deleteLyricsUpload,
  fetchLyricsFile,
  type LyricsFormat,
  type LyricsList,
  type LyricsSource,
  type Song,
  uploadLyrics,
} from "../api";
import { lyricsQuery } from "../api/queries";
import { fileNameFrom, lyricsFormatOf, saveBlob } from "../lib/download";
import { type TranslationKey, useI18n } from "../state/i18n";
import { mayCorrectTracks, useLibraryScope } from "../state/library-scope";
import { Icon } from "./icons";

const SOURCE_LABELS: Record<LyricsSource, TranslationKey> = {
  upload_ttml: "lyrics.source.uploadTtml",
  upload_lrc: "lyrics.source.uploadLrc",
  upload_txt: "lyrics.source.uploadTxt",
  sidecar_lrc: "lyrics.source.sidecarLrc",
  sidecar_text: "lyrics.source.sidecarText",
  embedded: "lyrics.source.embedded",
  lrclib: "lyrics.source.lrclib",
};

const UPLOADED: Partial<Record<LyricsSource, LyricsFormat>> = {
  upload_ttml: "ttml",
  upload_lrc: "lrc",
  upload_txt: "txt",
};

/**
 * The lyrics of a track in its properties panel (RFC-012 decision 6).
 *
 * Every sheet the track has, in the order the server chose — the first is the
 * one every reader shows — each with where it came from. Anyone may download
 * the first; owners and managers may also upload a `.lrc`, `.txt` or `.ttml`
 * and take an upload back. Nothing here touches the audio file.
 */
export function LyricsSection({
  song,
  list,
  known,
}: {
  song: Song;
  list: LyricsList | undefined;
  /** Whether the lyrics have been answered, so "none" is not said early. */
  known: boolean;
}) {
  const { t } = useI18n();
  const client = useQueryClient();
  const { libraries } = useLibraryScope();
  const mayUpload = mayCorrectTracks(
    libraries.find((library) => library.id === song.library_id),
  );
  const picker = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const sheets = list?.structuredLyrics ?? [];

  // The new list is the server's answer to the write itself; setting it first
  // keeps the panel from flashing the old one while everything else re-reads.
  async function settle(next?: LyricsList) {
    if (next) client.setQueryData(lyricsQuery(song.id).queryKey, next);
    await client.invalidateQueries({ queryKey: lyricsQuery(song.id).queryKey });
  }

  async function upload(file: File) {
    const format = lyricsFormatOf(file.name);
    if (!format) {
      setProblem(t("lyrics.unsupported"));
      return;
    }
    setBusy(true);
    setProblem(null);
    try {
      await settle(await uploadLyrics(song.id, format, file));
    } catch (error) {
      setProblem(
        t(
          error instanceof ApiError && error.status === 422
            ? "lyrics.empty"
            : error instanceof ApiError && error.status === 413
              ? "lyrics.tooLarge"
              : "lyrics.failed",
        ),
      );
    } finally {
      setBusy(false);
    }
  }

  async function remove(format: LyricsFormat) {
    setBusy(true);
    setProblem(null);
    try {
      await deleteLyricsUpload(song.id, format);
      await settle();
    } catch {
      setProblem(t("lyrics.failed"));
    } finally {
      setBusy(false);
    }
  }

  async function download() {
    setProblem(null);
    try {
      const { blob, name } = await fetchLyricsFile(song.id);
      saveBlob(blob, fileNameFrom(name) ?? `${song.title}.txt`);
    } catch {
      setProblem(t("lyrics.downloadFailed"));
    }
  }

  return (
    <section className="lyrics-section">
      <h3>{t("properties.lyrics")}</h3>
      <dl className="property-list">
        {sheets.map((sheet, index) => {
          const uploaded = sheet.source ? UPLOADED[sheet.source] : undefined;
          return (
            <div
              className="property-row"
              // Sheets have no id; what tells two apart is where they came from
              // and what they hold.
              key={`${sheet.source ?? ""}|${sheet.lang}|${sheet.synced}|${sheet.line[0]?.value ?? ""}`}
            >
              <dt>
                {index === 0
                  ? t("lyrics.shown")
                  : t("properties.lyricsSheet", { number: index + 1 })}
              </dt>
              <dd>
                <span>
                  {t(sheet.synced ? "properties.synced" : "properties.plain", {
                    lang: sheet.lang,
                  })}
                </span>
                {sheet.source ? (
                  <small className="muted">
                    {t(SOURCE_LABELS[sheet.source])}
                  </small>
                ) : null}
                {uploaded && mayUpload ? (
                  <button
                    type="button"
                    className="lyrics-remove"
                    disabled={busy}
                    onClick={() => void remove(uploaded)}
                  >
                    {t("lyrics.remove", { format: uploaded.toUpperCase() })}
                  </button>
                ) : null}
              </dd>
            </div>
          );
        })}
        {known && sheets.length === 0 ? (
          <div className="property-row">
            <dt>{t("properties.lyricsSheets")}</dt>
            <dd>{t("properties.noLyrics")}</dd>
          </div>
        ) : null}
      </dl>
      <div className="lyrics-actions">
        {sheets.length > 0 ? (
          <button
            type="button"
            className="lyrics-action"
            onClick={() => void download()}
          >
            <Icon name="download" size={16} />
            {t("lyrics.download")}
          </button>
        ) : null}
        {mayUpload ? (
          <>
            <button
              type="button"
              className="lyrics-action"
              disabled={busy}
              aria-busy={busy}
              onClick={() => picker.current?.click()}
            >
              <Icon name="upload" size={16} />
              {t("lyrics.upload")}
            </button>
            <input
              ref={picker}
              type="file"
              accept=".lrc,.txt,.ttml"
              hidden
              aria-label={t("lyrics.upload")}
              onChange={(event) => {
                const file = event.target.files?.[0];
                // Cleared, so choosing the same file again is a change too.
                event.target.value = "";
                if (file) void upload(file);
              }}
            />
          </>
        ) : null}
      </div>
      {problem ? (
        <p className="error" role="alert">
          {problem}
        </p>
      ) : null}
    </section>
  );
}
