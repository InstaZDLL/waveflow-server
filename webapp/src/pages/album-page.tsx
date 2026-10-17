import { useQuery } from "@tanstack/react-query";
import { albumQuery } from "../api/queries";
import { Artwork } from "../components/artwork";
import { Loading } from "../components/page-shell";
import { SongTable } from "../components/song-table";
import { useI18n } from "../state/i18n";

export function AlbumPage({ albumId }: { albumId: string }) {
  const { t } = useI18n();
  const { data: value, error } = useQuery(albumQuery(albumId));
  if (!value) return <Loading error={error} />;
  return (
    <section>
      <header className="detail-header">
        <Artwork
          artworkId={value.artwork_hash}
          title={value.title}
          className="large"
        />
        <div>
          <span className="eyebrow">{t("common.album")}</span>
          <h2>{value.title}</h2>
          <p className="muted">
            {value.artist ?? t("common.variousArtists")}
            {value.year ? ` · ${value.year}` : ""} ·{" "}
            {t("common.tracks", { count: value.songs.length })}
          </p>
        </div>
      </header>
      <SongTable songs={value.songs} numbering="track" />
    </section>
  );
}
