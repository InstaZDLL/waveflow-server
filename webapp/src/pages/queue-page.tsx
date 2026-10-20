import { useMemo } from "react";

import { formatDuration } from "../api";
import { Artwork } from "../components/artwork";
import { EmptyState, PageHeader } from "../components/page-shell";
import { usePlayer } from "../player/player";
import { useI18n } from "../state/i18n";

export function QueuePage() {
  const player = usePlayer();
  const { t } = useI18n();
  const queueKeys = useMemo(() => {
    const occurrences = new Map<string, number>();
    return player.queue.map((song) => {
      const occurrence = occurrences.get(song.id) ?? 0;
      occurrences.set(song.id, occurrence + 1);
      return `${song.id}-${occurrence}`;
    });
  }, [player.queue]);
  return (
    <section>
      <PageHeader
        title={t("nav.queue")}
        detail={t("queue.detail", { count: player.queue.length })}
      />
      {player.queue.length ? (
        <>
          <div className="actions section-actions">
            <button type="button" onClick={() => player.play(player.queue, 0)}>
              {t("queue.playStart")}
            </button>
            <button type="button" className="danger" onClick={player.clear}>
              {t("queue.clear")}
            </button>
          </div>
          <ol className="queue-list">
            {player.queue.map((song, position) => (
              <li
                key={queueKeys[position]}
                className={player.index === position ? "active" : undefined}
              >
                <button
                  type="button"
                  className="queue-track"
                  onClick={() => player.play(player.queue, position)}
                >
                  <Artwork
                    artworkId={song.artwork_hash}
                    title={song.title}
                    className="queue-cover"
                  />
                  <span className="queue-copy">
                    <strong className="queue-song-title">{song.title}</strong>
                    <small className="queue-artist muted">
                      {song.artist ?? t("common.unknownArtist")}
                    </small>
                  </span>
                </button>
                <span className="queue-duration muted">
                  {formatDuration(song.duration_ms)}
                </span>
                <button
                  type="button"
                  className="queue-remove"
                  onClick={() => player.remove(position)}
                  aria-label={`${t("queue.remove")}: ${song.title}`}
                >
                  {t("queue.remove")}
                </button>
              </li>
            ))}
          </ol>
        </>
      ) : (
        <EmptyState message={t("queue.empty")} />
      )}
    </section>
  );
}
