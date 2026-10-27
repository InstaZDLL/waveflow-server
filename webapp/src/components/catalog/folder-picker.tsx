import { useQuery } from "@tanstack/react-query";
import { useEffect, useId, useRef, useState } from "react";

import { folderOutline, isWithin } from "../../api";
import { filedTreeQuery } from "../../api/queries";
import { useI18n } from "../../state/i18n";
import { Icon } from "../layout/icons";

/**
 * Choose a filed folder (RFC-012 decision 2): the keyboard's way to file a
 * track or move a folder, where the folders page also takes a drag.
 *
 * A native `<dialog>` opened with `showModal`, as Properties is: the focus
 * trap, Escape and the inert page behind come from the platform. The tree is
 * one list, indented by depth; the first entry is the root, which means
 * **Unfiled** for a track and **the top level** for a folder — the caller says
 * which. `exclude` leaves out a folder and everything beneath it: a folder
 * cannot be moved under itself, and the server would refuse it anyway.
 */
export function FolderPicker({
  libraryId,
  title,
  rootLabel,
  current,
  exclude,
  nested = true,
  onPick,
  onClose,
}: {
  libraryId: string;
  title: string;
  rootLabel: string;
  /**
   * Where the thing is now, marked and not offered as a destination: a folder
   * id, `null` for the root, or `undefined` when it is not known.
   */
  current?: string | null;
  exclude?: string;
  /**
   * Whether the folders sit beneath the root entry. True for moving a folder,
   * where "Top level" holds them; false for filing a track, where "Unfiled"
   * is one more choice beside them, not their parent.
   */
  nested?: boolean;
  /** Rejects to keep the dialog open and say the move failed. */
  onPick: (folderId: string | null) => Promise<void>;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const { data, error } = useQuery(filedTreeQuery(libraryId));
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    const node = dialog.current;
    if (node && !node.open) node.showModal();
  }, []);

  const folders = data?.folders ?? [];
  const outline = folderOutline(folders).filter(
    ({ folder }) => !exclude || !isWithin(folders, folder.id, exclude),
  );

  async function pick(folderId: string | null) {
    setBusy(true);
    setFailed(false);
    try {
      await onPick(folderId);
      dialog.current?.close();
    } catch {
      setFailed(true);
    } finally {
      setBusy(false);
    }
  }

  const entry = (folderId: string | null, label: string, depth: number) => {
    const here = current !== undefined && current === folderId;
    return (
      <li key={folderId ?? "root"}>
        <button
          type="button"
          className="folder-pick"
          style={{ paddingInlineStart: `calc(var(--space-3) + ${depth}rem)` }}
          disabled={busy || here}
          aria-current={here ? "location" : undefined}
          onClick={() => void pick(folderId)}
        >
          <Icon name="folder" size={16} />
          <span>{label}</span>
          {here ? <small className="muted">{t("filed.here")}</small> : null}
        </button>
      </li>
    );
  };

  return (
    // biome-ignore lint/a11y/useKeyWithClickEvents: Escape closes a <dialog> natively
    <dialog
      ref={dialog}
      className="folder-picker"
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
      {error ? (
        <p className="error" role="alert">
          {t("filed.loadError")}
        </p>
      ) : (
        <ul className="folder-pick-list" aria-label={t("filed.destinations")}>
          {entry(null, rootLabel, 0)}
          {outline.map(({ folder, depth }) =>
            entry(folder.id, folder.name, depth + (nested ? 1 : 0)),
          )}
        </ul>
      )}
      {failed ? (
        <p className="error" role="alert">
          {t("filed.moveFailed")}
        </p>
      ) : null}
    </dialog>
  );
}
