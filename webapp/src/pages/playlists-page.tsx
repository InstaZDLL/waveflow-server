import { useQuery } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";

import {
  appendToPlaylist,
  createPlaylist,
  deletePlaylist,
  type Playlist,
} from "../api";
import { playlistsQuery } from "../api/queries";
import {
  EmptyState,
  Loading,
  PageHeader,
  useReload,
} from "../components/page-shell";
import {
  downloadPlaylist,
  PlaylistImportPanel,
} from "../components/playlist-import";
import { SongTable } from "../components/song-table";
import { usePlayer } from "../player/player";
import { useI18n } from "../state/i18n";

export function PlaylistsPage() {
  const player = usePlayer();
  const { t } = useI18n();
  const reload = useReload(playlistsQuery().queryKey);
  const [name, setName] = useState("");
  const [mutationError, setMutationError] = useState<string | null>(null);
  const { data: value, error } = useQuery(playlistsQuery());

  async function create(event: FormEvent) {
    event.preventDefault();
    setMutationError(null);
    try {
      await createPlaylist(name);
      setName("");
      reload();
    } catch {
      setMutationError(t("playlists.createError"));
    }
  }

  async function addQueue(playlist: Playlist) {
    if (!player.queue.length) return;
    setMutationError(null);
    try {
      await appendToPlaylist(
        playlist.id,
        player.queue.map((song) => song.id),
      );
      reload();
    } catch {
      setMutationError(t("playlists.queueError"));
    }
  }

  async function removePlaylist(id: string) {
    setMutationError(null);
    try {
      await deletePlaylist(id);
      reload();
    } catch {
      setMutationError(t("playlists.deleteError"));
    }
  }

  async function save(playlist: Playlist) {
    setMutationError(null);
    try {
      await downloadPlaylist(playlist.id, playlist.name);
    } catch {
      setMutationError(t("playlists.exportError"));
    }
  }

  if (!value) return <Loading error={error} />;
  return (
    <section>
      <PageHeader
        kicker={t("nav.group.library")}
        title={t("nav.playlists")}
        detail={t("playlists.detail", { count: value.length })}
      />
      <form className="inline-form" onSubmit={(event) => void create(event)}>
        <input
          value={name}
          onChange={(event) => setName(event.target.value)}
          placeholder={t("playlists.newName")}
          aria-label={t("playlists.newName")}
          required
        />
        <button type="submit">{t("playlists.create")}</button>
      </form>
      <PlaylistImportPanel onImported={reload} />
      {mutationError ? <p className="error">{mutationError}</p> : null}
      {value.length ? (
        <div className="stack">
          {value.map((playlist) => (
            <article className="collection" key={playlist.id}>
              <header className="collection-header">
                <div>
                  <h3>{playlist.name}</h3>
                  <span className="muted">
                    {t("common.tracks", { count: playlist.songs.length })}
                  </span>
                </div>
                <div className="actions">
                  <button
                    type="button"
                    onClick={() => player.play(playlist.songs, 0)}
                    disabled={!playlist.songs.length}
                  >
                    {t("playlists.play")}
                  </button>
                  <button
                    type="button"
                    onClick={() => void addQueue(playlist)}
                    disabled={!player.queue.length}
                  >
                    {t("playlists.addQueue")}
                  </button>
                  <button
                    type="button"
                    onClick={() => void save(playlist)}
                    disabled={!playlist.songs.length}
                  >
                    {t("playlists.export")}
                  </button>
                  <button
                    type="button"
                    className="danger"
                    onClick={() => void removePlaylist(playlist.id)}
                  >
                    {t("common.delete")}
                  </button>
                </div>
              </header>
              {playlist.songs.length ? (
                <SongTable songs={playlist.songs} />
              ) : (
                <p className="muted">{t("playlists.emptyOne")}</p>
              )}
            </article>
          ))}
        </div>
      ) : (
        <EmptyState message={t("playlists.empty")} />
      )}
    </section>
  );
}
