import type { Song } from "../api";
import { usePlayer } from "../player/player";
import { useI18n } from "../state/i18n";

/** Plays a set of songs, or adds them to the queue, from a page header. */
export function PlaySetActions({ songs }: { songs: Song[] }) {
  const player = usePlayer();
  const { t } = useI18n();
  return (
    <div className="actions">
      <button
        type="button"
        onClick={() => player.play(songs, 0)}
        disabled={!songs.length}
      >
        {t("playlists.play")}
      </button>
      <button
        type="button"
        onClick={() => player.enqueue(songs)}
        disabled={!songs.length}
      >
        {t("playlists.addQueue")}
      </button>
    </div>
  );
}
