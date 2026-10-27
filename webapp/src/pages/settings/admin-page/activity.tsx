// What the server is doing now: a scan's progress, who is listening.

import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";

import { type ScanJob, watchScan } from "../../../api";
import { nowPlayingQuery } from "../../../api/queries";
import { type TranslationKey, useI18n } from "../../../state/i18n";

/**
 * The statuses `scan_job.status` holds. Looked up rather than cast: an unknown
 * key makes `translate` read a property of `undefined` and takes the page down,
 * so a status this table does not know is shown as it came.
 */
const SCAN_STATUS: Record<string, TranslationKey> = {
  queued: "scan.queued",
  running: "scan.running",
  completed: "scan.completed",
  failed: "scan.failed",
};

/**
 * A scan while it runs. The admin screen used to start one and say nothing
 * more; the progress stream has been there all along.
 */
export function ScanProgress({ scanId }: { scanId: string }) {
  const { t } = useI18n();
  const [job, setJob] = useState<ScanJob | null>(null);
  const [lost, setLost] = useState(false);

  useEffect(() => {
    setJob(null);
    setLost(false);
    // The snapshot arrives on the stream itself, so there is nothing to fetch
    // first; a stream that cannot open leaves the panel on its last reading.
    return watchScan(scanId, setJob, () => setLost(true));
  }, [scanId]);

  // A stream that cannot open has to say so: without this the panel waited on
  // a first reading that was never coming, for as long as the page stayed up.
  if (!job && lost) {
    return (
      <p className="error" role="alert">
        {t("scan.lost")}
      </p>
    );
  }
  if (!job) return <p className="muted">{t("scan.connecting")}</p>;
  const done = job.status !== "running" && job.status !== "queued";
  const share = job.total_files > 0 ? job.processed_files / job.total_files : 0;
  return (
    <div className="scan-progress">
      <div className="scan-line">
        <strong>
          {SCAN_STATUS[job.status] ? t(SCAN_STATUS[job.status]) : job.status}
        </strong>
        <span className="muted">
          {t("scan.counted", {
            processed: job.processed_files,
            total: job.total_files,
          })}
        </span>
      </div>
      <progress
        max={job.total_files || 1}
        value={job.processed_files}
        aria-label={t("scan.progress")}
      >
        {Math.round(share * 100)}%
      </progress>
      <p className="muted scan-tally">
        {t("scan.tally", {
          added: job.added,
          updated: job.updated,
          moved: job.moved,
          skipped: job.skipped,
          errors: job.errors,
        })}
      </p>
      {/* What the walk left out (RFC-021): only when there is something to
          say, since most libraries have neither. */}
      {job.excluded > 0 || job.skipped_links > 0 ? (
        <p className="muted scan-tally">
          {t("scan.leftOut", {
            excluded: job.excluded,
            links: job.skipped_links,
          })}
        </p>
      ) : null}
      {/* The path is the only thing here that moves fast enough to read as
          liveness, and it is what tells a long scan from a stuck one. */}
      {!done && job.current_path ? (
        <p className="muted scan-path">{job.current_path}</p>
      ) : null}
      {job.message ? <p className="error">{job.message}</p> : null}
    </div>
  );
}

/** Who is listening to what, right now, across the accounts one can see. */
export function NowPlayingPanel() {
  const { t } = useI18n();
  // No stream exists for this one, so it is polled, on the interval the query
  // carries. A query keeps the reading it has while it refetches, so the panel
  // no longer has to hold the previous one in a state of its own to avoid
  // flashing "nobody is listening" every thirty seconds.
  const { data: entries } = useQuery(nowPlayingQuery());

  return (
    <article className="admin-panel now-playing-panel">
      <h3>{t("admin.nowPlaying")}</h3>
      {entries?.length ? (
        <ul className="list resource-list">
          {entries.map((entry) => (
            <li key={`${entry.username}-${entry.song.id}`}>
              <div>
                <strong>{entry.song.title}</strong>
                <small className="muted">
                  {entry.song.artist ?? t("common.unknownArtist")}
                </small>
              </div>
              <span className="muted">{entry.username}</span>
            </li>
          ))}
        </ul>
      ) : (
        <p className="muted">{t("admin.nobodyListening")}</p>
      )}
    </article>
  );
}
