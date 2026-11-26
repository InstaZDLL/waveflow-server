import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import {
  appendToPlaylist,
  createPlaylist,
  MAX_PLACEMENTS,
  placeTracks,
  type Song,
} from "../../../api";
import { FolderPicker } from "../../../components/catalog/folder-picker";
import { Icon } from "../../../components/layout/icons";
import { PlaylistPicker } from "../../../components/playlists/playlist-picker";
import { usePlayer } from "../../../player/player";
import { useI18n } from "../../../state/i18n";

/**
 * What can be done with the rows picked in a folder, all at once.
 *
 * Moving a whole folder's worth one track at a time, from each row's menu,
 * is what this replaces; so is making a playlist of a folder by playing it
 * and saving the queue. Shown only while something is picked, above the
 * list, where the picked rows are.
 */
export function SelectionBar({
  libraryId,
  songs,
  mayFile,
  folder,
  onDone,
  onMoved,
  onClear,
}: {
  libraryId: string;
  /** The picked tracks, in the list's order. */
  songs: Song[];
  /** Whether this account may file tracks in this library. */
  mayFile: boolean;
  /** The folder the tracks are listed in: `null` for Unfiled. */
  folder: string | null;
  /** Says what was done, once it is. */
  onDone: (said: string) => void;
  /**
   * The tracks a move filed elsewhere, once it has: only those leave the
   * selection, so rows picked while a slow move ran stay picked.
   */
  onMoved: (ids: string[]) => void;
  onClear: () => void;
}) {
  const { t } = useI18n();
  const player = usePlayer();
  const client = useQueryClient();
  const [dialog, setDialog] = useState<"folder" | "playlist" | null>(null);
  const ids = songs.map((song) => song.id);
  const count = ids.length;

  return (
    <div
      className="selection-bar"
      role="toolbar"
      aria-label={t("selection.label")}
    >
      <strong aria-live="polite">{t("selection.count", { count })}</strong>
      <button type="button" onClick={() => player.play(songs, 0)}>
        <Icon name="play" size={16} />
        {t("selection.play")}
      </button>
      <button type="button" onClick={() => player.enqueue(songs)}>
        <Icon name="queue" size={16} />
        {t("selection.queue")}
      </button>
      {mayFile ? (
        <button type="button" onClick={() => setDialog("folder")}>
          <Icon name="folder" size={16} />
          {t("selection.move")}
        </button>
      ) : null}
      <button type="button" onClick={() => setDialog("playlist")}>
        <Icon name="playlists" size={16} />
        {t("selection.playlist")}
      </button>
      <button type="button" className="link" onClick={onClear}>
        {t("selection.clear")}
      </button>
      {dialog === "folder" ? (
        <FolderPicker
          libraryId={libraryId}
          title={t("selection.moveTitle", { count })}
          rootLabel={t("filed.unfiled")}
          current={folder}
          nested={false}
          onPick={async (target) => {
            // One request per batch the server takes, each all or nothing.
            // Past 500 tracks a later batch can fail after earlier ones were
            // filed: those are shown where they now are, and leave the
            // selection, whatever the rest did.
            const moved: string[] = [];
            let filed = 0;
            try {
              for (let at = 0; at < ids.length; at += MAX_PLACEMENTS) {
                const batch = ids.slice(at, at + MAX_PLACEMENTS);
                filed += await placeTracks(libraryId, batch, target);
                moved.push(...batch);
              }
            } finally {
              await Promise.all([
                client.invalidateQueries({ queryKey: ["filed-tree"] }),
                client.invalidateQueries({ queryKey: ["folder-tracks"] }),
              ]);
              if (moved.length) onMoved(moved);
            }
            onDone(t("selection.moved", { count: filed }));
          }}
          onClose={() => setDialog(null)}
        />
      ) : null}
      {dialog === "playlist" ? (
        <PlaylistPicker
          title={t("selection.playlistTitle", { count })}
          onPick={async (playlist) => {
            await appendToPlaylist(playlist.id, ids);
            await client.invalidateQueries({ queryKey: ["playlists"] });
            onDone(t("selection.added", { count, name: playlist.name }));
          }}
          onCreate={async (name) => {
            await createPlaylist(name, ids);
            await client.invalidateQueries({ queryKey: ["playlists"] });
            onDone(t("selection.added", { count, name }));
          }}
          onClose={() => setDialog(null)}
        />
      ) : null}
    </div>
  );
}
