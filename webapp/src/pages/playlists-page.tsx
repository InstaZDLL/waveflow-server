import { useQuery } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";

import {
  appendToPlaylist,
  createPlaylist,
  currentUser,
  deletePlaylist,
  type Playlist,
  setPlaylistPublic,
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
import { SmartPlaylistsSection } from "../components/smart-playlists";
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

  async function publish(playlist: Playlist, isPublic: boolean) {
    setMutationError(null);
    try {
      await setPlaylistPublic(playlist.id, isPublic);
      reload();
    } catch {
      setMutationError(t("playlists.publicError"));
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
  // The list answers this account's playlists, then the public ones of every
  // other account (RFC-020). Only the first are this account's to change: the
  // server answers an edit of the others 404, so the page offers none.
  const me = currentUser()?.id;
  const own = value.filter((playlist) => playlist.owner_id === me);
  const others = value.filter((playlist) => playlist.owner_id !== me);

  const card = (playlist: Playlist, mine: boolean) => (
    <article className="collection" key={playlist.id}>
      <header className="collection-header">
        <div>
          <h3>{playlist.name}</h3>
          <span className="muted">
            {t("common.tracks", { count: playlist.songs.length })}
            {mine
              ? null
              : ` · ${t("playlists.by", { owner: playlist.owner_username })}`}
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
          {mine ? (
            <button
              type="button"
              onClick={() => void addQueue(playlist)}
              disabled={!player.queue.length}
            >
              {t("playlists.addQueue")}
            </button>
          ) : null}
          <button
            type="button"
            onClick={() => void save(playlist)}
            disabled={!playlist.songs.length}
          >
            {t("playlists.export")}
          </button>
          {mine ? (
            <>
              <label className="checkbox-label">
                <input
                  type="checkbox"
                  checked={playlist.public}
                  onChange={(event) =>
                    void publish(playlist, event.target.checked)
                  }
                />
                {t("playlists.public")}
              </label>
              <button
                type="button"
                className="danger"
                onClick={() => void removePlaylist(playlist.id)}
              >
                {t("common.delete")}
              </button>
            </>
          ) : null}
        </div>
      </header>
      {playlist.songs.length ? (
        <SongTable songs={playlist.songs} />
      ) : (
        <p className="muted">{t("playlists.emptyOne")}</p>
      )}
    </article>
  );

  return (
    <section>
      <PageHeader
        kicker={t("nav.group.library")}
        title={t("nav.playlists")}
        detail={t("playlists.detail", { count: own.length })}
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
      {own.length ? (
        <div className="stack">
          {own.map((playlist) => card(playlist, true))}
        </div>
      ) : (
        <EmptyState message={t("playlists.empty")} />
      )}
      {others.length ? (
        <section className="stack" aria-labelledby="other-playlists">
          <header>
            <h3 id="other-playlists">{t("playlists.others")}</h3>
            <p className="muted">{t("playlists.othersDetail")}</p>
          </header>
          {others.map((playlist) => card(playlist, false))}
        </section>
      ) : null}
      {/* Rules rather than a track list, beside the ordinary ones (RFC-025). */}
      <SmartPlaylistsSection />
    </section>
  );
}
