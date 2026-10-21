import { useQuery } from "@tanstack/react-query";
import { historyTracksQuery } from "../api/queries";
import { EmptyState, Loading, PageHeader } from "../components/page-shell";
import { PlaySetActions } from "../components/play-set-actions";
import { SongTable } from "../components/song-table";
import { useI18n } from "../state/i18n";

/**
 * How many distinct tracks the recently-played screen resolves.
 *
 * Each one costs a request — `GET /history` answers plays, not songs — and they
 * all leave at once. Asking for 200 plays and resolving every distinct track
 * among them could mean 200 round trips for a list nobody reads to the bottom
 * of. The window stays wide so that a session spent replaying one album still
 * reaches back past it; only what is shown is capped.
 */
const HISTORY_TRACKS = 50;

export function HistoryPage() {
  const { t } = useI18n();
  const { data: value, error } = useQuery(historyTracksQuery(HISTORY_TRACKS));
  if (!value) return <Loading error={error} />;
  return (
    <section>
      <PageHeader
        kicker={t("nav.group.library")}
        title={t("nav.history")}
        detail={t("history.detail", { count: value.length })}
      >
        {value.length ? <PlaySetActions songs={value} /> : null}
      </PageHeader>
      {value.length ? (
        <SongTable songs={value} />
      ) : (
        <EmptyState message={t("history.empty")} />
      )}
    </section>
  );
}
