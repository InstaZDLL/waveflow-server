import {
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useEffect, useMemo, useRef, useState } from "react";

import {
  QUALITY_VERDICTS,
  type QualityAnalysis,
  type QualityTrack,
  type QualityVerdict,
  requestQualityAnalysis,
} from "../api";
import { qualitySummaryQuery, qualityTracksQuery } from "../api/queries";
import { Artwork } from "../components/artwork";
import { containerName, kilohertz } from "../components/file-quality";
import { Loading, PageHeader } from "../components/page-shell";
import { VERDICT_LABEL } from "../components/quality-section";
import { TrackProperties } from "../components/track-properties";
import { analysisIsWorking, highsStop, verdictTone } from "../lib/quality";
import { useI18n } from "../state/i18n";
import { mayCorrectTracks, useLibraryScope } from "../state/library-scope";

/** How long "Analyse again" waits for the second click, as Split does. */
const CONFIRM_MS = 4000;

/**
 * How often the counts are asked again while the analysis works. Short
 * enough that the count reads as moving: a file takes a fraction of a second
 * on a release build, and the reader is watching for exactly that.
 */
const REFRESH_MS = 3_000;

/**
 * The formats a reader filters by, as the scanner names them. ALAC is named
 * `AAC` by the scanner, so it is filtered with AAC.
 */
const FORMATS = [
  "FLAC",
  "WAV",
  "AIFF",
  "APE",
  "WavPack",
  "AAC",
  "MP3",
  "Vorbis",
  "Opus",
];

export function isQualityVerdict(value: unknown): value is QualityVerdict {
  return QUALITY_VERDICTS.includes(value as QualityVerdict);
}

/**
 * The real quality of a library's files (RFC-016 decision 10): measured from
 * the audio, not from what the container claims. Every member sees it, since
 * every member already sees the container's own facts; asking for the whole
 * library to be analysed again is for its owners and managers.
 *
 * Worst first, so the probable fakes open the page. The verdict is not
 * repeated on the other lists of the client, for the desktop's reason: a
 * dull recording taken for an MP3 would carry the accusation on every row.
 */
export function QualityPage({
  verdict,
}: {
  verdict: QualityVerdict | undefined;
}) {
  const { t, locale } = useI18n();
  const { active, ready } = useLibraryScope();
  const libraryId = active?.id ?? "";
  const [codec, setCodec] = useState("");
  const summary = useQuery({
    ...qualitySummaryQuery(libraryId),
    enabled: active !== null,
    // Asked again on every arrival, with the list: the analysis moves in the
    // background, and a header held from a visit thirty seconds ago would
    // contradict the rows drawn fresh under it.
    staleTime: 0,
    refetchInterval: (query) =>
      analysisIsWorking(query.state.data?.analysis?.state) ? REFRESH_MS : false,
  });
  const client = useQueryClient();
  // And the counts follow the list: a filter asks for a list the cache does
  // not hold yet, so the header is asked again beside it. Not on the first
  // render, which has just asked for both.
  const filtered = useRef(false);
  useEffect(() => {
    void verdict;
    void codec;
    if (!filtered.current) {
      filtered.current = true;
      return;
    }
    void client.invalidateQueries({
      queryKey: ["quality", libraryId, "summary"],
    });
  }, [verdict, codec, libraryId, client]);
  // The list follows the counts: a file measured in the background moves. Only
  // when the count moves, not on its first answer, which the list has just
  // been asked alongside.
  const measured = summary.data?.measured;
  const lastMeasured = useRef<number | undefined>(undefined);
  useEffect(() => {
    const previous = lastMeasured.current;
    lastMeasured.current = measured;
    if (previous !== undefined && measured !== previous) {
      void client.invalidateQueries({
        queryKey: ["quality", libraryId, "tracks"],
      });
    }
  }, [measured, libraryId, client]);

  const count = (key: QualityVerdict) =>
    summary.data?.verdicts.find((entry) => entry.verdict === key)?.count ?? 0;
  const number = new Intl.NumberFormat(locale);
  const detail = !summary.data
    ? undefined
    : !summary.data.enabled
      ? t("quality.disabled")
      : summary.data.lossless === 0
        ? t("quality.noLossless")
        : t("quality.progress", {
            measured: number.format(summary.data.measured),
            lossless: number.format(summary.data.lossless),
          });

  const header = (
    <PageHeader
      kicker={t("nav.maintenance")}
      title={t("nav.quality")}
      detail={detail}
    >
      {active &&
      mayCorrectTracks(active) &&
      summary.isFetchedAfterMount &&
      summary.data?.analysis?.state === "idle" ? (
        <AnalyseAgain libraryId={libraryId} />
      ) : null}
    </PageHeader>
  );

  if (ready && !active) {
    return (
      <section>
        {header}
        <p className="muted">{t("folders.noLibrary")}</p>
      </section>
    );
  }

  return (
    <section className="quality-page">
      {header}
      {summary.data?.analysis ? (
        <AnalysisStatus
          analysis={summary.data.analysis}
          measured={summary.data.measured}
          lossless={summary.data.lossless}
        />
      ) : null}
      <p className="muted quality-help">{t("quality.help")}</p>
      {!summary.data ? (
        <Loading error={summary.error} />
      ) : (
        <>
          <ul className="quality-verdicts" aria-label={t("quality.filters")}>
            <li>
              <Link
                to="/maintenance/quality"
                search={{ verdict: undefined }}
                className="attention-category"
                aria-current={verdict === undefined ? "page" : undefined}
              >
                <span>{t("quality.all")}</span>
              </Link>
            </li>
            {QUALITY_VERDICTS.map((key) => (
              <li key={key}>
                <Link
                  to="/maintenance/quality"
                  search={{ verdict: key }}
                  className={`attention-category${count(key) === 0 ? " empty" : ""}`}
                  aria-current={verdict === key ? "page" : undefined}
                >
                  <span>{t(VERDICT_LABEL[key])}</span>
                  <small className="attention-count">
                    {number.format(count(key))}
                  </small>
                </Link>
              </li>
            ))}
          </ul>
          <label className="quality-format">
            <span>{t("quality.format")}</span>
            <select
              value={codec}
              onChange={(event) => setCodec(event.target.value)}
            >
              <option value="">{t("quality.allFormats")}</option>
              {FORMATS.map((format) => (
                <option key={format} value={format}>
                  {format}
                </option>
              ))}
            </select>
          </label>
          <QualityList
            libraryId={libraryId}
            verdict={verdict}
            codec={codec || undefined}
          />
        </>
      )}
    </section>
  );
}

/**
 * Where the analysis stands, in one line: what it is doing, and the count
 * moving under it while it works. Announced politely, so a screen reader
 * hears the state change without being interrupted on every file.
 */
function AnalysisStatus({
  analysis,
  measured,
  lossless,
}: {
  analysis: QualityAnalysis;
  measured: number;
  lossless: number;
}) {
  const { t, locale } = useI18n();
  const number = new Intl.NumberFormat(locale);
  const when = (instant: number) =>
    new Intl.DateTimeFormat(locale, {
      dateStyle: "medium",
      timeStyle: "short",
    }).format(instant);
  const text = (() => {
    switch (analysis.state) {
      case "running":
        return t("quality.status.running", {
          measured: number.format(measured),
          lossless: number.format(lossless),
          inFlight: number.format(analysis.in_flight),
        });
      case "waiting_for_scan":
        return t("quality.status.waitingForScan");
      case "queued":
        return t("quality.status.queued");
      case "off":
        return t("quality.disabled");
      default:
        return analysis.last_measured_at === null
          ? t("quality.status.idleNever")
          : t("quality.status.idle", { when: when(analysis.last_measured_at) });
    }
  })();
  return (
    <p
      className={`quality-status ${analysis.state}`}
      role="status"
      aria-live="polite"
    >
      {analysisIsWorking(analysis.state) ? (
        <span className="quality-status-dot" aria-hidden="true" />
      ) : null}
      {text}
    </p>
  );
}

function QualityList({
  libraryId,
  verdict,
  codec,
}: {
  libraryId: string;
  verdict: QualityVerdict | undefined;
  codec: string | undefined;
}) {
  const { t, locale } = useI18n();
  const tracks = useInfiniteQuery({
    ...qualityTracksQuery(libraryId, { verdict, codec }),
    // Asked again on every arrival, as the header is: see `QualityPage`.
    staleTime: 0,
  });
  const songs = useMemo(() => tracks.data?.pages.flat() ?? [], [tracks.data]);
  const [opened, setOpened] = useState<QualityTrack | null>(null);
  const returnTo = useRef<HTMLElement | null>(null);
  if (!tracks.data) return <Loading error={tracks.error} />;
  if (!songs.length) return <p className="muted">{t("quality.empty")}</p>;
  const kilo = new Intl.NumberFormat(locale, { maximumFractionDigits: 1 });
  const facts = (song: QualityTrack) =>
    [
      containerName(song),
      typeof song.bit_depth === "number" ? `${song.bit_depth} bit` : null,
      typeof song.sample_rate === "number"
        ? `${kilohertz(song.sample_rate, locale)} kHz`
        : null,
      typeof song.bitrate === "number" ? `${song.bitrate} kbps` : null,
    ]
      .filter((fact): fact is string => fact !== null)
      .join(" · ");
  const highs = (song: QualityTrack) => {
    const found = highsStop(song.quality.verdict, song.quality.cutoff_hz);
    if (found === null) return "—";
    return found.kind === "fullBand"
      ? t("quality.fullBand")
      : t("quality.khz", { value: kilo.format(found.hz / 1000) });
  };

  return (
    <>
      <div className="songs-scroll">
        <table className="songs quality-table">
          <thead>
            <tr>
              <th scope="col">{t("quality.column.track")}</th>
              <th scope="col">{t("quality.column.container")}</th>
              <th scope="col">{t("quality.column.highs")}</th>
              <th scope="col">{t("quality.column.verdict")}</th>
            </tr>
          </thead>
          <tbody>
            {songs.map((song) => (
              <tr key={song.id}>
                <td>
                  <button
                    type="button"
                    className="link song-title"
                    onClick={(event) => {
                      returnTo.current = event.currentTarget;
                      setOpened(song);
                    }}
                  >
                    <Artwork
                      artworkId={song.artwork_hash}
                      title={song.title}
                      className="song-cover"
                    />
                    <span>
                      {song.title}
                      <small>
                        {[song.artist, song.album].filter(Boolean).join(" · ")}
                      </small>
                    </span>
                  </button>
                </td>
                <td className="muted">{facts(song)}</td>
                <td className="quality-highs">{highs(song)}</td>
                <td>
                  <span
                    className={`quality-verdict ${verdictTone(song.quality.verdict)}`}
                    data-verdict={song.quality.verdict}
                  >
                    {t(VERDICT_LABEL[song.quality.verdict])}
                  </span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {tracks.hasNextPage ? (
        <div className="actions">
          <button
            type="button"
            onClick={() => tracks.fetchNextPage()}
            disabled={tracks.isFetchingNextPage}
          >
            {tracks.isFetchingNextPage
              ? t("songs.loading")
              : t("songs.loadMore")}
          </button>
        </div>
      ) : null}
      {opened ? (
        <TrackProperties
          song={opened}
          onClose={() => {
            setOpened(null);
            returnTo.current?.focus();
          }}
        />
      ) : null}
    </>
  );
}

/**
 * Drops the library's measurements and queues every file again. The first
 * click arms it and the second, within four seconds, confirms, as a split
 * does on "Needs attention": the whole library goes back through FFmpeg.
 */
function AnalyseAgain({ libraryId }: { libraryId: string }) {
  const { t } = useI18n();
  const client = useQueryClient();
  const [armed, setArmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<{ error: boolean; text: string } | null>(
    null,
  );
  useEffect(() => {
    if (!armed) return;
    const timer = window.setTimeout(() => setArmed(false), CONFIRM_MS);
    return () => window.clearTimeout(timer);
  }, [armed]);

  async function confirm() {
    setBusy(true);
    try {
      await requestQualityAnalysis(libraryId);
      setNotice({ error: false, text: t("quality.requested") });
      await client.invalidateQueries({ queryKey: ["quality", libraryId] });
      await client.invalidateQueries({ queryKey: ["track-quality"] });
      await client.invalidateQueries({ queryKey: ["attention", libraryId] });
    } catch {
      setNotice({ error: true, text: t("quality.requestFailed") });
    } finally {
      setBusy(false);
      setArmed(false);
    }
  }

  return (
    <div className="quality-analyse">
      <button
        type="button"
        className={armed ? "action-primary" : undefined}
        disabled={busy}
        onClick={() => (armed ? void confirm() : setArmed(true))}
      >
        {armed ? t("quality.confirmAnalyse") : t("quality.analyseAgain")}
      </button>
      {notice ? (
        <p
          className={`quality-analyse-notice ${notice.error ? "error" : "muted"}`}
          role={notice.error ? "alert" : "status"}
        >
          {notice.text}
        </p>
      ) : null}
    </div>
  );
}
