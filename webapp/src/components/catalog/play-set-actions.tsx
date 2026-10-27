import type { Song } from "../../api";
import { usePlayer } from "../../player/player";
import { useI18n } from "../../state/i18n";
import { Icon } from "../layout/icons";

/** Plays a set of songs, or adds them to the queue, from a page header. */
export function PlaySetActions({ songs }: { songs: Song[] }) {
  const player = usePlayer();
  const { t } = useI18n();
  return (
    <div className="actions">
      <button
        className="action-primary"
        type="button"
        onClick={() => player.play(songs, 0)}
        disabled={!songs.length}
      >
        <Icon name="play" size={17} />
        {t("playlists.play")}
      </button>
      <button
        className="action-secondary"
        type="button"
        onClick={() => player.enqueue(songs)}
        disabled={!songs.length}
      >
        <Icon name="queue" size={17} />
        {t("playlists.addQueue")}
      </button>
    </div>
  );
}
