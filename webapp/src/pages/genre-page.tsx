import { useQuery } from "@tanstack/react-query";
import { useState } from "react";

import { listRandomSongs } from "../api";
import { genreSongsQuery } from "../api/queries";
import { EmptyState, Loading, PageHeader } from "../components/page-shell";
import { SongTable } from "../components/song-table";
import { usePlayer } from "../player/player";
import { useI18n } from "../state/i18n";
import { useScopeId } from "../state/library-scope";

export function GenrePage({ genre }: { genre: string }) {
  const player = usePlayer();
  const { t } = useI18n();
  const [drawing, setDrawing] = useState(false);
  const scope = useScopeId();
  const { data: value, error } = useQuery(genreSongsQuery(genre, scope));

  /**
   * A shuffle of this genre, drawn by the server. It is a separate route from
   * the listing below rather than a client-side shuffle of it, because
   * `/songs/random` samples the whole genre and this page holds only what it
   * has paged in.
   */
  async function drawRandom() {
    setDrawing(true);
    try {
      const songs = await listRandomSongs(100, genre, scope);
      if (songs.length) player.play(songs, 0);
    } catch {
      // The listing below is still playable; nothing to recover from.
    } finally {
      setDrawing(false);
    }
  }

  if (!value) return <Loading error={error} />;
  return (
    <section>
      <PageHeader
        title={genre}
        detail={t("common.tracks", { count: value.length })}
      >
        <div className="actions">
          <button
            type="button"
            onClick={() => player.play(value, 0)}
            disabled={!value.length}
          >
            {t("playlists.play")}
          </button>
          <button
            type="button"
            onClick={() => void drawRandom()}
            disabled={drawing || !value.length}
          >
            {t("random.fromGenre")}
          </button>
        </div>
      </PageHeader>
      {value.length ? (
        <SongTable songs={value} />
      ) : (
        <EmptyState message={t("genres.emptyOne")} />
      )}
    </section>
  );
}
