// One library's panels: who may see it, its settings and its deletion.

import { useQuery } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";

import {
  ApiError,
  deleteLibrary,
  type Library,
  removeLibraryMember,
  setLibraryMember,
  type User,
  updateLibrary,
} from "../../../api";
import {
  administrationQuery,
  libraryMembersQuery,
  librarySettingsQuery,
} from "../../../api/queries";
import { useReload, Waiting } from "../../../components/layout/page-shell";
import { type TranslationKey, useI18n } from "../../../state/i18n";

/**
 * Who may see one library, and in what standing.
 *
 * Its own disclosure per library, for the reason the token panel has one: the
 * admin screen renders it once per library, and loading on mount would ask for
 * every membership list every time the page opened.
 *
 * The owner is shown and cannot be changed from here — the route refuses
 * `owner` outright, so offering it would be offering a refusal.
 */
export function LibraryMembersPanel({
  libraryId,
  users,
}: {
  libraryId: string;
  users: User[];
}) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [failed, setFailed] = useState(false);
  const reload = useReload(libraryMembersQuery(libraryId, true).queryKey);
  const { data: value, error } = useQuery(libraryMembersQuery(libraryId, open));

  async function grant(userId: string, role: "manager" | "listener") {
    setFailed(false);
    try {
      await setLibraryMember(libraryId, userId, role);
      reload();
    } catch {
      setFailed(true);
    }
  }

  async function revoke(userId: string) {
    setFailed(false);
    try {
      await removeLibraryMember(libraryId, userId);
      reload();
    } catch {
      setFailed(true);
    }
  }

  // `value ?? []` conflated three states, and two of them are not an empty
  // list. An empty list is not even reachable: every library has at least its
  // owner, and the service answers 404 rather than nothing. So `null` means
  // "not known yet" — either in flight or failed — and standing in an empty
  // array for it made `outside` every account on the server, offering access
  // to people who already had it, under the error saying the list could not be
  // read.
  const members = value;
  const outside = members
    ? users.filter(
        (user) => !members.some((member) => member.user_id === user.id),
      )
    : [];
  return (
    <article className="admin-panel token-panel">
      <h3>
        <button
          type="button"
          aria-expanded={open}
          onClick={() => setOpen((value) => !value)}
        >
          {t("admin.members")}
        </button>
      </h3>
      {open ? (
        <>
          {failed ? <p className="error">{t("admin.memberError")}</p> : null}
          {error ? (
            <p className="error" role="alert">
              {t("common.loadError")}
            </p>
          ) : !members ? (
            <Waiting />
          ) : (
            <ul className="list resource-list">
              {members.map((member) => (
                <li key={member.user_id}>
                  <div>
                    <strong>{member.username}</strong>
                    <small className="muted">{member.role}</small>
                  </div>
                  {member.role === "owner" ? (
                    <span className="muted">{t("admin.memberOwner")}</span>
                  ) : (
                    <div className="actions">
                      <label className="control">
                        <span>{t("admin.memberRole")}</span>
                        <select
                          value={member.role}
                          aria-label={`${t("admin.memberRole")}: ${member.username}`}
                          onChange={(event) =>
                            void grant(
                              member.user_id,
                              event.target.value as "manager" | "listener",
                            )
                          }
                        >
                          <option value="listener">listener</option>
                          <option value="manager">manager</option>
                        </select>
                      </label>
                      <button
                        type="button"
                        className="danger"
                        onClick={() => void revoke(member.user_id)}
                        aria-label={`${t("admin.memberRemove")}: ${member.username}`}
                      >
                        {t("admin.memberRemove")}
                      </button>
                    </div>
                  )}
                </li>
              ))}
            </ul>
          )}
          {outside.length ? (
            <label className="control">
              <span>{t("admin.memberAdd")}</span>
              <select
                value=""
                aria-label={t("admin.memberAdd")}
                onChange={(event) =>
                  event.target.value &&
                  void grant(event.target.value, "listener")
                }
              >
                <option value="">{t("admin.memberChoose")}</option>
                {outside.map((user) => (
                  <option key={user.id} value={user.id}>
                    {user.username}
                  </option>
                ))}
              </select>
            </label>
          ) : null}
        </>
      ) : null}
    </article>
  );
}

/**
 * One library's name, folder and links setting, and its deletion (RFC-021
 * decision 5). A disclosure of its own per library, for the reason the
 * membership panel has one: the settings, which carry the folder on the
 * server, are asked for only once it is open.
 *
 * Deleting asks for the library's name to be typed, and the server checks it
 * again: a deletion takes every track, album and artist out of the catalogue
 * for every member, with their playlist entries, favourites and plays.
 */
export function LibrarySettingsPanel({
  library,
  onScan,
  onDeleted,
}: {
  library: Library;
  onScan: (scanId: string) => void;
  onDeleted: (message: string) => void;
}) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const reloadSettings = useReload(
    librarySettingsQuery(library.id, true).queryKey,
  );
  const reloadAdministration = useReload(administrationQuery().queryKey);
  const { data: settings, error } = useQuery(
    librarySettingsQuery(library.id, open),
  );
  // `null` until edited: the form shows what the server holds until then.
  const [name, setName] = useState<string | null>(null);
  const [path, setPath] = useState<string | null>(null);
  const [follow, setFollow] = useState<boolean | null>(null);
  const [watch, setWatch] = useState<boolean | null>(null);
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<{ error: boolean; text: string } | null>(
    null,
  );

  function refusal(error: unknown, fallback: TranslationKey): string {
    return error instanceof ApiError && error.status === 409
      ? t("admin.saveBusy")
      : t(fallback);
  }

  async function save(event: FormEvent) {
    event.preventDefault();
    if (!settings) return;
    const update: {
      name?: string;
      path?: string;
      follow_links?: boolean;
      watch?: boolean;
    } = {};
    if (name !== null && name.trim() !== settings.name) update.name = name;
    if (path !== null && path !== settings.path) update.path = path;
    if (follow !== null && follow !== settings.follow_links) {
      update.follow_links = follow;
    }
    if (watch !== null && watch !== settings.watch) update.watch = watch;
    if (Object.keys(update).length === 0) return;
    setBusy(true);
    setNotice(null);
    try {
      const result = await updateLibrary(library.id, update);
      setNotice({
        error: false,
        text: result.scan_id
          ? t("admin.savedScan", { id: result.scan_id })
          : t("admin.saved"),
      });
      if (result.scan_id) onScan(result.scan_id);
      setName(null);
      setPath(null);
      setFollow(null);
      setWatch(null);
      reloadSettings();
      reloadAdministration();
    } catch (error) {
      setNotice({ error: true, text: refusal(error, "admin.saveError") });
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    setBusy(true);
    setNotice(null);
    try {
      const deleted = await deleteLibrary(library.id, confirm);
      onDeleted(
        t("admin.deleted", { name: library.name, tracks: deleted.tracks }),
      );
      reloadAdministration();
    } catch (error) {
      setNotice({ error: true, text: refusal(error, "admin.deleteError") });
      setBusy(false);
    }
  }

  return (
    <article className="admin-panel token-panel">
      <h3>
        <button
          type="button"
          aria-expanded={open}
          onClick={() => setOpen((value) => !value)}
        >
          {t("admin.manage")}
        </button>
      </h3>
      {open ? (
        error ? (
          <p className="error" role="alert">
            {t("common.loadError")}
          </p>
        ) : !settings ? (
          <Waiting />
        ) : (
          <>
            <form
              className="stacked-form"
              onSubmit={(event) => void save(event)}
            >
              <label className="control">
                <span>{t("admin.rename")}</span>
                <input
                  value={name ?? settings.name}
                  onChange={(event) => setName(event.target.value)}
                  required
                />
              </label>
              <label className="control">
                <span>{t("admin.path")}</span>
                <input
                  value={path ?? settings.path}
                  onChange={(event) => setPath(event.target.value)}
                  required
                />
              </label>
              <label className="toggle-row">
                <input
                  type="checkbox"
                  className="toggle-box"
                  checked={follow ?? settings.follow_links}
                  onChange={(event) => setFollow(event.target.checked)}
                />
                <span>{t("admin.followLinks")}</span>
              </label>
              <label className="toggle-row">
                <input
                  type="checkbox"
                  className="toggle-box"
                  checked={watch ?? settings.watch}
                  onChange={(event) => setWatch(event.target.checked)}
                />
                <span>{t("admin.watch")}</span>
              </label>
              <button type="submit" disabled={busy}>
                {t("admin.save")}
              </button>
            </form>
            {notice ? (
              <p
                className={notice.error ? "error" : "notice"}
                role={notice.error ? "alert" : "status"}
              >
                {notice.text}
              </p>
            ) : null}
            <div className="stacked-form danger-zone">
              <strong>{t("admin.delete")}</strong>
              <p className="muted danger-note">{t("admin.deleteNote")}</p>
              <input
                value={confirm}
                onChange={(event) => setConfirm(event.target.value)}
                placeholder={library.name}
                aria-label={t("admin.deleteConfirm")}
                autoComplete="off"
              />
              <button
                type="button"
                className="danger"
                disabled={busy || confirm !== library.name}
                onClick={() => void remove()}
              >
                {t("admin.deleteButton")}
              </button>
            </div>
          </>
        )
      ) : null}
    </article>
  );
}
