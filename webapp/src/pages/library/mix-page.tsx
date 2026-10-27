import { useQuery } from "@tanstack/react-query";

import { mixQuery } from "../../api/queries";
import { PlaySetActions } from "../../components/catalog/play-set-actions";
import { SongTable } from "../../components/catalog/song-table";
import { Loading, PageHeader } from "../../components/layout/page-shell";
import { mixTitle } from "../../lib/mixes";
import { useI18n } from "../../state/i18n";
import { useScopeId } from "../../state/library-scope";
import { useLocalDay } from "../../state/local-day";

/**
 * One mix and its tracks (RFC-024 decision 7). The server draws it once a
 * day, so the page shows the same tracks in the same order all day.
 */
export function MixPage({ mixId }: { mixId: string }) {
  const { t } = useI18n();
  const scope = useScopeId();
  const day = useLocalDay();
  const { data: mix, error } = useQuery(mixQuery(mixId, scope, day));
  if (!mix) return <Loading error={error} />;
  return (
    <section>
      <PageHeader
        kicker={t("mixes.title")}
        title={mixTitle(mix, t)}
        detail={t("mixes.detail", { count: mix.songs.length })}
      >
        <PlaySetActions songs={mix.songs} />
      </PageHeader>
      <SongTable songs={mix.songs} />
    </section>
  );
}
