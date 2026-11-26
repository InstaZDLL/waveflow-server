import { useQuery } from "@tanstack/react-query";
import { type FormEvent, useEffect, useId, useRef, useState } from "react";

import { currentUser, type Playlist } from "../../api";
import { playlistsQuery } from "../../api/queries";
import { useI18n } from "../../state/i18n";
import { Icon } from "../layout/icons";

/**
 * Choose a playlist to add tracks to, or name a new one to hold them.
 *
 * Only this account's own playlists: another account's public one is listed
 * beside them elsewhere, but only its owner may change it. A native `<dialog>`
 * opened with `showModal`, as the folder picker is, and drawn with its styles.
 */
export function PlaylistPicker({
  title,
  onPick,
  onCreate,
  onClose,
}: {
  title: string;
  /** Rejects to keep the dialog open and say the tracks were not added. */
  onPick: (playlist: Playlist) => Promise<void>;
  /** Rejects likewise. */
  onCreate: (name: string) => Promise<void>;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const nameId = useId();
  const { data, error } = useQuery(playlistsQuery());
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const [name, setName] = useState("");
  const me = currentUser()?.id;
  const own = (data ?? []).filter((playlist) => playlist.owner_id === me);

  useEffect(() => {
    const node = dialog.current;
    if (node && !node.open) node.showModal();
  }, []);

  async function run(write: () => Promise<void>) {
    setBusy(true);
    setFailed(false);
    try {
      await write();
      dialog.current?.close();
    } catch {
      setFailed(true);
    } finally {
      setBusy(false);
    }
  }

  function create(event: FormEvent) {
    event.preventDefault();
    const trimmed = name.trim();
    if (trimmed) void run(() => onCreate(trimmed));
  }

  return (
    // biome-ignore lint/a11y/useKeyWithClickEvents: Escape closes a <dialog> natively
    <dialog
      ref={dialog}
      className="folder-picker playlist-picker"
      aria-labelledby={titleId}
      onClose={onClose}
      onClick={(event) => {
        // A click on the backdrop lands on the dialog itself, outside its box.
        const node = dialog.current;
        if (!node || event.target !== node) return;
        const box = node.getBoundingClientRect();
        const inside =
          event.clientX >= box.left &&
          event.clientX <= box.right &&
          event.clientY >= box.top &&
          event.clientY <= box.bottom;
        if (!inside) node.close();
      }}
    >
      <header>
        <h2 id={titleId}>{title}</h2>
        <button
          type="button"
          className="properties-close"
          aria-label={t("properties.close")}
          onClick={() => dialog.current?.close()}
        >
          <Icon name="close" />
        </button>
      </header>
      <form className="playlist-picker-new" onSubmit={create}>
        <label htmlFor={nameId}>{t("playlistPicker.new")}</label>
        <input
          id={nameId}
          value={name}
          onChange={(event) => setName(event.target.value)}
          maxLength={200}
          placeholder={t("playlistPicker.name")}
        />
        <button
          type="submit"
          className="action-primary"
          disabled={busy || !name.trim()}
        >
          {t("playlistPicker.create")}
        </button>
      </form>
      {error ? (
        <p className="error" role="alert">
          {t("playlistPicker.loadError")}
        </p>
      ) : data && !own.length ? (
        <p className="muted">{t("playlistPicker.none")}</p>
      ) : (
        <ul
          className="folder-pick-list"
          aria-label={t("playlistPicker.existing")}
        >
          {own.map((playlist) => (
            <li key={playlist.id}>
              <button
                type="button"
                className="folder-pick"
                disabled={busy}
                onClick={() => void run(() => onPick(playlist))}
              >
                <Icon name="playlists" size={16} />
                <span>{playlist.name}</span>
                <small className="muted">
                  {t("common.tracks", { count: playlist.songs.length })}
                </small>
              </button>
            </li>
          ))}
        </ul>
      )}
      {failed ? (
        <p className="error" role="alert">
          {t("playlistPicker.failed")}
        </p>
      ) : null}
    </dialog>
  );
}
