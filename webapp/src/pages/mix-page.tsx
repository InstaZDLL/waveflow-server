import { useQuery } from "@tanstack/react-query";

import { mixQuery } from "../api/queries";
import { Loading, PageHeader } from "../components/page-shell";
import { PlaySetActions } from "../components/play-set-actions";
import { SongTable } from "../components/song-table";
import { mixTitle } from "../lib/mixes";
import { useI18n } from "../state/i18n";
import { useScopeId } from "../state/library-scope";

/**
 * One mix and its tracks (RFC-024 decision 7). The server draws it once a
 * day, so the page shows the same tracks in the same order all day.
 */
export function MixPage({ mixId }: { mixId: string }) {
  const { t } = useI18n();
  const scope = useScopeId();
  const { data: mix, error } = useQuery(mixQuery(mixId, scope));
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
