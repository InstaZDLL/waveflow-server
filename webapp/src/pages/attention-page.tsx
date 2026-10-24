import {
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { useEffect, useMemo, useState } from "react";

import {
  ApiError,
  type AttentionCategory,
  type AttentionTrack,
  type CreditCandidate,
  type CreditDecision,
  decideCredit,
  removeCreditDecision,
  type TrackCategory,
} from "../api";
import {
  attentionSummaryQuery,
  attentionTracksQuery,
  creditCandidatesQuery,
  creditDecisionsQuery,
} from "../api/queries";
import { Loading, PageHeader } from "../components/page-shell";
import { SongTable } from "../components/song-table";
import { type TranslationKey, useI18n } from "../state/i18n";
import { mayCorrectTracks, useLibraryScope } from "../state/library-scope";

/** The page's lists, in the desktop's order, with what each is called. */
const CATEGORIES: Array<{ key: AttentionCategory; label: TranslationKey }> = [
  { key: "split_credits", label: "attention.split_credits" },
  { key: "missing_artist", label: "attention.missing_artist" },
  { key: "missing_album", label: "attention.missing_album" },
  { key: "missing_year", label: "attention.missing_year" },
  { key: "missing_track_number", label: "attention.missing_track_number" },
  { key: "missing_cover", label: "attention.missing_cover" },
  { key: "album_year_conflict", label: "attention.album_year_conflict" },
  {
    key: "compilation_no_album_artist",
    label: "attention.compilation_no_album_artist",
  },
  { key: "duplicate_track_number", label: "attention.duplicate_track_number" },
  { key: "track_number_gap", label: "attention.track_number_gap" },
  { key: "probable_duplicate", label: "attention.probable_duplicate" },
  {
    key: "probable_fake_lossless",
    label: "attention.probable_fake_lossless",
  },
];

/** How long a Split waits for the second click, as on the desktop. */
const CONFIRM_MS = 4000;

export function isAttentionCategory(
  value: unknown,
): value is AttentionCategory {
  return CATEGORIES.some((entry) => entry.key === value);
}

/**
 * "Needs attention" (RFC-015 decision 8): what a library's owners and
 * managers are asked to look at. The joined credits waiting for a decision —
 * split into the artists they name, or kept whole — and the desktop's track
 * categories the server can compute. A track category offers no action of
 * its own: it opens its tracks in the song table, whose menu holds the editor.
 *
 * The rail shows no count. A number that moves after every scan, in a corner
 * of every page, would be a notification nobody asked for; the sum is said
 * here, at the top of the page it belongs to.
 */
export function AttentionPage({
  category,
}: {
  category: AttentionCategory | undefined;
}) {
  const { t } = useI18n();
  const { active, ready } = useLibraryScope();
  const libraryId = active?.id ?? "";
  const may = mayCorrectTracks(active ?? undefined);
  const summary = useQuery({
    ...attentionSummaryQuery(libraryId),
    enabled: active !== null && may,
  });
  const counts = useMemo(
    () =>
      new Map(
        (summary.data ?? []).map((entry) => [entry.category, entry.count]),
      ),
    [summary.data],
  );
  const total = summary.data?.reduce((sum, entry) => sum + entry.count, 0);
  // Without one in the address, the first list holding something is open —
  // and is then written into the address. Recomputed on every answer, the
  // choice would move under the hand that just emptied the list: a split
  // takes the last credit off "Artists to split", and the page would jump to
  // the next list, notice and all.
  const open: AttentionCategory | undefined =
    category ??
    CATEGORIES.find((entry) => (counts.get(entry.key) ?? 0) > 0)?.key;
  const navigate = useNavigate();
  useEffect(() => {
    if (category === undefined && open !== undefined) {
      void navigate({
        to: "/attention",
        search: { category: open },
        replace: true,
      });
    }
  }, [category, open, navigate]);

  const header = (
    <PageHeader
      kicker={t("nav.group.library")}
      title={t("nav.attention")}
      detail={
        total === undefined
          ? undefined
          : total === 0
            ? t("attention.nothing")
            : t("attention.total", { count: total })
      }
    />
  );

  if (ready && (!active || !may)) {
    return (
      <section>
        {header}
        <p className="muted">
          {t(active ? "attention.notYours" : "folders.noLibrary")}
        </p>
      </section>
    );
  }

  return (
    <section className="attention-page">
      {header}
      {!summary.data ? (
        <Loading error={summary.error} />
      ) : (
        <>
          <ul
            className="attention-categories"
            aria-label={t("attention.lists")}
          >
            {CATEGORIES.map((entry) => {
              const count = counts.get(entry.key) ?? 0;
              return (
                <li key={entry.key}>
                  <Link
                    to="/attention"
                    search={{ category: entry.key }}
                    className={`attention-category${count === 0 ? " empty" : ""}`}
                    aria-current={open === entry.key ? "page" : undefined}
                  >
                    <span>{t(entry.label)}</span>
                    <small className="attention-count">{count}</small>
                  </Link>
                </li>
              );
            })}
          </ul>
          {open === "split_credits" ? (
            <CreditsPanel libraryId={libraryId} />
          ) : open ? (
            <TracksPanel libraryId={libraryId} category={open} />
          ) : (
            <p className="muted">{t("attention.nothing")}</p>
          )}
        </>
      )}
    </section>
  );
}

function TracksPanel({
  libraryId,
  category,
}: {
  libraryId: string;
  category: TrackCategory;
}) {
  const { t } = useI18n();
  const tracks = useInfiniteQuery(attentionTracksQuery(libraryId, category));
  const songs = useMemo(() => tracks.data?.pages.flat() ?? [], [tracks.data]);
  // Probable duplicates arrive group by group; each is drawn as its own table
  // so a group reads as one recording. A group a page boundary cut in two is
  // joined again here, because adjacent rows of one group are one group.
  const groups = useMemo(() => {
    const grouped: Array<{ group: number; songs: AttentionTrack[] }> = [];
    for (const song of songs) {
      const group = song.duplicate_group ?? 0;
      const last = grouped.at(-1);
      if (last && last.group === group) last.songs.push(song);
      else grouped.push({ group, songs: [song] });
    }
    return grouped;
  }, [songs]);
  if (!tracks.data) return <Loading error={tracks.error} />;
  if (!songs.length) return <p className="muted">{t("attention.empty")}</p>;
  return (
    <>
      <p className="muted attention-note">{t("attention.tracksNote")}</p>
      {category === "probable_duplicate" ? (
        groups.map((entry) => (
          <section key={entry.group} className="attention-group">
            <h3>{t("attention.duplicateGroup", { group: entry.group })}</h3>
            <SongTable songs={entry.songs} />
          </section>
        ))
      ) : (
        <SongTable songs={songs} />
      )}
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
    </>
  );
}

type Notice = { error: boolean; text: string } | null;

function CreditsPanel({ libraryId }: { libraryId: string }) {
  const { t } = useI18n();
  const client = useQueryClient();
  const [view, setView] = useState<"pending" | "decided">("pending");
  const candidates = useQuery(creditCandidatesQuery(libraryId));
  const decisions = useQuery({
    ...creditDecisionsQuery(libraryId),
    enabled: view === "decided",
  });
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<Notice>(null);

  /**
   * A decision moves credits everywhere an artist is read, so everything is
   * re-read rather than a list of caches that might be missing one.
   */
  async function attempt(write: () => Promise<unknown>, said: string) {
    setBusy(true);
    setNotice(null);
    // Re-read whatever the answer: a refusal is usually another owner having
    // decided first — the credit is decided, or its artist is gone — and the
    // row on screen is what is out of date.
    try {
      await write();
      setNotice({ error: false, text: said });
    } catch (error) {
      setNotice({
        error: true,
        text: t(
          error instanceof ApiError &&
            (error.status === 409 || error.status === 404)
            ? "attention.alreadyDecided"
            : "attention.failed",
        ),
      });
    } finally {
      await client.invalidateQueries();
      setBusy(false);
    }
  }

  return (
    <div className="attention-credits">
      <p className="muted attention-note">{t("attention.creditsNote")}</p>
      <div className="attention-views">
        <button
          type="button"
          aria-pressed={view === "pending"}
          onClick={() => setView("pending")}
        >
          {t("attention.pending")}
        </button>
        <button
          type="button"
          aria-pressed={view === "decided"}
          onClick={() => setView("decided")}
        >
          {t("attention.decided")}
        </button>
      </div>
      {notice ? (
        <p
          className={notice.error ? "error" : "muted attention-notice"}
          role={notice.error ? "alert" : "status"}
        >
          {notice.text}
        </p>
      ) : null}
      {view === "pending" ? (
        !candidates.data ? (
          <Loading error={candidates.error} />
        ) : candidates.data.length ? (
          <ul className="attention-rows">
            {candidates.data.map((candidate) => (
              <CandidateRow
                key={candidate.artist_id}
                candidate={candidate}
                busy={busy}
                onSplit={() =>
                  attempt(
                    () => decideCredit(libraryId, candidate.artist_id, "split"),
                    t("attention.splitDone", { name: candidate.name }),
                  )
                }
                onKeep={() =>
                  attempt(
                    () => decideCredit(libraryId, candidate.artist_id, "keep"),
                    t("attention.keepDone", { name: candidate.name }),
                  )
                }
              />
            ))}
          </ul>
        ) : (
          <p className="muted">{t("attention.noCredits")}</p>
        )
      ) : !decisions.data ? (
        <Loading error={decisions.error} />
      ) : decisions.data.length ? (
        <ul className="attention-rows">
          {decisions.data.map((decision) => (
            <DecisionRow
              key={decision.id}
              decision={decision}
              busy={busy}
              onUndo={() =>
                attempt(
                  () => removeCreditDecision(libraryId, decision.id),
                  t("attention.undone", { name: decision.credit }),
                )
              }
            />
          ))}
        </ul>
      ) : (
        <p className="muted">{t("attention.noDecisions")}</p>
      )}
    </div>
  );
}

function CandidateRow({
  candidate,
  busy,
  onSplit,
  onKeep,
}: {
  candidate: CreditCandidate;
  busy: boolean;
  onSplit: () => Promise<void>;
  onKeep: () => Promise<void>;
}) {
  const { t } = useI18n();
  // Split asks twice: the first click arms it, a second within four seconds
  // fires. Keep is undone as easily as it is done, so it asks once.
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    if (!armed) return;
    const timer = window.setTimeout(() => setArmed(false), CONFIRM_MS);
    return () => window.clearTimeout(timer);
  }, [armed]);
  return (
    <li className="attention-row">
      <div className="attention-row-main">
        <Link
          to="/artists/$artistId"
          params={{ artistId: candidate.artist_id }}
          className="attention-credit"
        >
          {candidate.name}
        </Link>
        <small className="muted">
          {t("common.tracks", { count: candidate.track_count })}
        </small>
      </div>
      <ul
        className="attention-fragments"
        aria-label={t("attention.fragments", { name: candidate.name })}
      >
        {candidate.fragments.map((fragment) => (
          <li key={fragment.name}>
            {fragment.artist_id ? (
              <Link
                to="/artists/$artistId"
                params={{ artistId: fragment.artist_id }}
                className="attention-fragment known"
                title={t("attention.knownFragment")}
              >
                {fragment.name}
              </Link>
            ) : (
              <span className="attention-fragment">{fragment.name}</span>
            )}
          </li>
        ))}
      </ul>
      <div className="attention-actions">
        <button
          type="button"
          className={armed ? "action-primary" : undefined}
          disabled={busy}
          onClick={() => {
            if (armed) {
              setArmed(false);
              void onSplit();
            } else {
              setArmed(true);
            }
          }}
        >
          {armed
            ? t("attention.confirmSplit", {
                count: candidate.fragments.length,
              })
            : t("attention.split")}
        </button>
        <button type="button" disabled={busy} onClick={() => void onKeep()}>
          {t("attention.keep")}
        </button>
      </div>
    </li>
  );
}

function DecisionRow({
  decision,
  busy,
  onUndo,
}: {
  decision: CreditDecision;
  busy: boolean;
  onUndo: () => Promise<void>;
}) {
  const { t, locale } = useI18n();
  const when = new Date(decision.decided_at).toLocaleDateString(locale);
  return (
    <li className="attention-row">
      <div className="attention-row-main">
        <span className="attention-credit">{decision.credit}</span>
        <small className="muted">
          {decision.decision === "split"
            ? t("attention.decidedSplit", {
                parts: (decision.parts ?? []).join(" · "),
              })
            : t("attention.decidedKeep")}
        </small>
        <small className="muted">
          {decision.decided_by
            ? t("attention.decidedBy", { who: decision.decided_by, when })
            : when}
        </small>
      </div>
      <div className="attention-actions">
        <button type="button" disabled={busy} onClick={() => void onUndo()}>
          {t("attention.undo")}
        </button>
      </div>
    </li>
  );
}
