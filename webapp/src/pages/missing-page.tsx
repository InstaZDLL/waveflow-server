import { useInfiniteQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";

import {
  ApiError,
  currentUser,
  purgeUnavailableTracks,
  restorePurgedTracks,
} from "../api";
import { purgedTracksQuery, unavailableTracksQuery } from "../api/queries";
import { Loading, PageHeader } from "../components/page-shell";
import { useI18n } from "../state/i18n";
import { mayManageMissing, useLibraryScope } from "../state/library-scope";

/** How long a purge waits for its second click, as Split does. */
const CONFIRM_MS = 4000;

/**
 * "Missing files" (RFC-021 decision 4): the tracks a library's scans can no
 * longer find, oldest first, for its owners and managers.
 *
 * Each row says what purging it would cost — playlist entries, favourites and
 * plays, across every account — because that is what a purge takes, and it
 * does not come back. A purge is two clicks: the first arms it, the second
 * within four seconds runs it. The server removes only what is still missing
 * when it runs, so a file that came back in between keeps its track.
 */
export function MissingPage() {
  const { t, locale } = useI18n();
  const { active, ready } = useLibraryScope();
  const libraryId = active?.id ?? "";
  const may = mayManageMissing(active ?? undefined, currentUser()?.role);
  const missing = useInfiniteQuery({
    ...unavailableTracksQuery(libraryId),
    enabled: active !== null && may,
    staleTime: 0,
  });
  // What a purge took out of sight and can still give back (RFC-021).
  const purged = useInfiniteQuery({
    ...purgedTracksQuery(libraryId),
    enabled: active !== null && may,
    staleTime: 0,
  });
  const purgedTracks = useMemo(
    () => purged.data?.pages.flatMap((page) => page.tracks) ?? [],
    [purged.data],
  );
  const purgedTotal = purged.data?.pages[0]?.total ?? 0;
  const client = useQueryClient();
  const tracks = useMemo(
    () => missing.data?.pages.flatMap((page) => page.tracks) ?? [],
    [missing.data],
  );
  const total = missing.data?.pages[0]?.total;
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [armed, setArmed] = useState<"selected" | "all" | null>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<{ error: boolean; text: string } | null>(
    null,
  );
  useEffect(() => {
    if (!armed) return;
    const timer = window.setTimeout(() => setArmed(null), CONFIRM_MS);
    return () => window.clearTimeout(timer);
  }, [armed]);
  // A selection belongs to one library's list.
  useEffect(() => {
    void libraryId;
    setSelected(new Set());
    setArmed(null);
  }, [libraryId]);

  const number = new Intl.NumberFormat(locale);
  const day = new Intl.DateTimeFormat(locale, { dateStyle: "medium" });

  const header = (
    <PageHeader
      kicker={t("nav.group.library")}
      title={t("nav.missing")}
      detail={
        total === undefined
          ? undefined
          : total === 0
            ? t("missing.nothing")
            : t("missing.total", { count: total })
      }
    />
  );

  if (ready && (!active || !may)) {
    return (
      <section>
        {header}
        <p className="muted">
          {t(active ? "missing.notYours" : "folders.noLibrary")}
        </p>
      </section>
    );
  }

  function toggle(id: string) {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
    setArmed(null);
  }

  async function purge(which: "selected" | "all") {
    setBusy(true);
    setNotice(null);
    try {
      const outcome = await purgeUnavailableTracks(
        libraryId,
        which === "all" ? { all: true } : { track_ids: [...selected] },
      );
      setSelected(new Set());
      setNotice({
        error: false,
        text:
          outcome.restorable_until === null
            ? t("missing.purged", { count: outcome.purged })
            : t("missing.purgedUntil", {
                count: outcome.purged,
                date: day.format(new Date(outcome.restorable_until)),
              }),
      });
      // Albums and artists left with nothing went with them, anywhere they
      // were drawn.
      await client.invalidateQueries();
    } catch (error) {
      setNotice({
        error: true,
        text:
          error instanceof ApiError && error.status === 409
            ? t("missing.scanning")
            : t("missing.failed"),
      });
    } finally {
      setBusy(false);
      setArmed(null);
    }
  }

  async function restore(selection: { track_ids: string[] } | { all: true }) {
    setBusy(true);
    setNotice(null);
    try {
      const outcome = await restorePurgedTracks(libraryId, selection);
      setNotice({
        error: false,
        text: t("missing.restored", { count: outcome.restored }),
      });
      await client.invalidateQueries({ queryKey: ["missing", libraryId] });
    } catch {
      setNotice({ error: true, text: t("missing.restoreFailed") });
    } finally {
      setBusy(false);
    }
  }

  // What the armed button would purge, said on the button itself.
  const count = armed === "all" ? (total ?? 0) : selected.size;

  return (
    <section className="missing-page">
      {header}
      <p className="muted">{t("missing.note")}</p>
      {notice ? (
        <p
          className={notice.error ? "error" : "notice"}
          role={notice.error ? "alert" : "status"}
        >
          {notice.text}
        </p>
      ) : null}
      {!missing.data ? (
        <Loading error={missing.error} />
      ) : tracks.length === 0 ? null : (
        <>
          <div className="actions missing-actions">
            <button
              type="button"
              className={armed === "selected" ? "danger" : undefined}
              disabled={busy || selected.size === 0}
              onClick={() =>
                armed === "selected"
                  ? void purge("selected")
                  : setArmed("selected")
              }
            >
              {armed === "selected"
                ? t("missing.confirm", { count })
                : t("missing.purgeSelected", { count: selected.size })}
            </button>
            <button
              type="button"
              className={armed === "all" ? "danger" : undefined}
              disabled={busy}
              onClick={() =>
                armed === "all" ? void purge("all") : setArmed("all")
              }
            >
              {armed === "all"
                ? t("missing.confirm", { count })
                : t("missing.purgeAll")}
            </button>
          </div>
          <ul className="list resource-list missing-list">
            {tracks.map((track) => (
              <li key={track.id}>
                <label className="missing-row">
                  <input
                    type="checkbox"
                    checked={selected.has(track.id)}
                    onChange={() => toggle(track.id)}
                    aria-label={t("missing.select", { title: track.title })}
                  />
                  <div className="missing-text">
                    <strong>{track.title}</strong>
                    <small className="muted">
                      {[track.artist, track.album].filter(Boolean).join(" · ")}
                    </small>
                    <small className="muted missing-path">
                      {track.relative_path}
                    </small>
                  </div>
                </label>
                <div className="missing-facts">
                  <small className="muted">
                    {t("missing.since", {
                      date: day.format(new Date(track.unavailable_since)),
                    })}
                  </small>
                  <small className="muted">
                    {t("missing.cost", {
                      playlists: number.format(track.playlist_entries),
                      favorites: number.format(track.favorites),
                      plays: number.format(track.plays),
                    })}
                  </small>
                </div>
              </li>
            ))}
          </ul>
          {missing.hasNextPage ? (
            <button
              type="button"
              disabled={missing.isFetchingNextPage}
              onClick={() => void missing.fetchNextPage()}
            >
              {t("missing.more")}
            </button>
          ) : null}
        </>
      )}
      {purgedTracks.length ? (
        <section className="stack" aria-labelledby="purged-tracks">
          <header>
            <h3 id="purged-tracks">
              {t("missing.purgedTitle", { count: purgedTotal })}
            </h3>
            <p className="muted">{t("missing.purgedNote")}</p>
          </header>
          <div className="actions missing-actions">
            <button
              type="button"
              disabled={busy}
              onClick={() => void restore({ all: true })}
            >
              {t("missing.restoreAll")}
            </button>
          </div>
          <ul className="list resource-list missing-list">
            {purgedTracks.map((track) => (
              <li key={track.id}>
                <div className="missing-text">
                  <strong>{track.title}</strong>
                  <small className="muted">
                    {[track.artist, track.album].filter(Boolean).join(" · ")}
                  </small>
                  <small className="muted missing-path">
                    {track.relative_path}
                  </small>
                </div>
                <div className="missing-facts">
                  {track.deleted_after !== null ? (
                    <small className="muted">
                      {t("missing.deletedAfter", {
                        date: day.format(new Date(track.deleted_after)),
                      })}
                    </small>
                  ) : null}
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => void restore({ track_ids: [track.id] })}
                    aria-label={t("missing.restoreOne", { title: track.title })}
                  >
                    {t("missing.restore")}
                  </button>
                </div>
              </li>
            ))}
          </ul>
          {purged.hasNextPage ? (
            <button
              type="button"
              disabled={purged.isFetchingNextPage}
              onClick={() => void purged.fetchNextPage()}
            >
              {t("missing.more")}
            </button>
          ) : null}
        </section>
      ) : null}
    </section>
  );
}
