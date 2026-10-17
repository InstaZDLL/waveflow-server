import { useQuery } from "@tanstack/react-query";
import { randomSongsQuery } from "../api/queries";
import { EmptyState, Loading, PageHeader } from "../components/page-shell";
import { PlaySetActions } from "../components/play-set-actions";
import { SongTable } from "../components/song-table";
import { useI18n } from "../state/i18n";
import { useScopeId } from "../state/library-scope";

export function RandomPage() {
  const { t } = useI18n();
  const scope = useScopeId();
  const { data: value, error, refetch } = useQuery(randomSongsQuery(scope));
  if (!value) return <Loading error={error} />;
  return (
    <section>
      <PageHeader
        title={t("nav.random")}
        detail={t("random.detail", { count: value.length })}
      >
        <div className="actions">
          <PlaySetActions songs={value} />
          {/* A redraw is the same question asked again, not a different one,
              so it refetches rather than changing a key. The query holds
              nothing between visits — a shuffle that came back identical
              would be the cache working and the page failing. */}
          <button type="button" onClick={() => void refetch()}>
            {t("random.again")}
          </button>
        </div>
      </PageHeader>
      {value.length ? (
        <SongTable songs={value} />
      ) : (
        <EmptyState message={t("random.empty")} />
      )}
    </section>
  );
}
