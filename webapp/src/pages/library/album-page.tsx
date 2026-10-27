import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { formatDuration } from "../../api";
import { albumQuery, artistAlbumsQuery } from "../../api/queries";
import { Artwork } from "../../components/artwork/artwork";
import { SongTable } from "../../components/catalog/song-table";
import { Icon } from "../../components/layout/icons";
import { Loading } from "../../components/layout/page-shell";
import { usePlayer } from "../../player/player";
import { useI18n } from "../../state/i18n";

export function AlbumPage({ albumId }: { albumId: string }) {
  const { t } = useI18n();
  const player = usePlayer();
  const { data: value, error } = useQuery(albumQuery(albumId));
  // The artist's albums alone, not a library paged through to keep six of
  // them, and not the artist's page, which would ask a provider. Kept to the
  // album's own library below, whichever library is active.
  const { data: albums } = useQuery({
    ...artistAlbumsQuery(value?.artist_id ?? ""),
    enabled: Boolean(value?.artist_id),
  });
  if (!value) return <Loading error={error} />;
  const totalDuration = value.songs.reduce(
    (total, song) => total + song.duration_ms,
    0,
  );
  const related = albums
    ?.filter(
      (album) =>
        album.id !== value.id &&
        album.artist_id === value.artist_id &&
        album.library_id === value.library_id,
    )
    .slice(0, 6);

  return (
    <section className="album-page">
      <header className="album-hero">
        {value.artwork_hash ? (
          <Artwork
            artworkId={value.artwork_hash}
            title={value.title}
            className="album-hero-ambient"
          />
        ) : null}
        <div className="album-hero-inner">
          <Artwork
            artworkId={value.artwork_hash}
            title={value.title}
            className="album-hero-cover"
          />
          <div className="album-hero-copy">
            <span className="album-type">{t("common.album")}</span>
            <h2>{value.title}</h2>
            <p className="album-meta">
              {value.artist_id ? (
                <Link
                  to="/artists/$artistId"
                  params={{ artistId: value.artist_id }}
                  className="album-artist-link"
                >
                  {value.artist ?? t("common.variousArtists")}
                </Link>
              ) : (
                <strong>{value.artist ?? t("common.variousArtists")}</strong>
              )}
              {value.year ? <span>{value.year}</span> : null}
              <span>{t("common.tracks", { count: value.songs.length })}</span>
              {totalDuration > 0 ? (
                <span>{formatDuration(totalDuration)}</span>
              ) : null}
            </p>
          </div>
        </div>
      </header>
      <div className="album-content">
        <div className="album-actions">
          <button
            type="button"
            className="album-play"
            disabled={value.songs.length === 0}
            onClick={() => player.play(value.songs, 0)}
          >
            <Icon name="play" size={20} filled />
            {t("album.play")}
          </button>
          <button
            type="button"
            className="album-enqueue"
            disabled={value.songs.length === 0}
            onClick={() => player.enqueue(value.songs)}
          >
            <Icon name="queue" size={18} />
            {t("album.addQueue")}
          </button>
        </div>
        <SongTable songs={value.songs} numbering="track" variant="album" />
        {related?.length ? (
          <section
            className="album-related"
            aria-labelledby="album-related-title"
          >
            <h3 id="album-related-title">
              {t("album.moreBy", {
                artist: value.artist ?? t("common.variousArtists"),
              })}
            </h3>
            <div className="album-related-grid">
              {related.map((album) => (
                <Link
                  key={album.id}
                  to="/albums/$albumId"
                  params={{ albumId: album.id }}
                >
                  <Artwork artworkId={album.artwork_hash} title={album.title} />
                  <strong>{album.title}</strong>
                  {album.year ? <span>{album.year}</span> : null}
                </Link>
              ))}
            </div>
          </section>
        ) : null}
      </div>
    </section>
  );
}
