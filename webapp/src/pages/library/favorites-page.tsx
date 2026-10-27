import { useQuery } from "@tanstack/react-query";
import { favoriteTracksQuery } from "../../api/queries";
import { SongTable } from "../../components/catalog/song-table";
import {
  EmptyState,
  Loading,
  PageHeader,
} from "../../components/layout/page-shell";
import { useI18n } from "../../state/i18n";

export function FavoritesPage() {
  const { t } = useI18n();
  const { data: value, error } = useQuery(favoriteTracksQuery());
  if (!value) return <Loading error={error} />;
  return (
    <section>
      <PageHeader
        kicker={t("nav.group.library")}
        title={t("nav.favourites")}
        detail={t("favourites.detail", { count: value.length })}
      />
      {value.length ? (
        <SongTable songs={value} />
      ) : (
        <EmptyState message={t("favourites.empty")} />
      )}
    </section>
  );
}
