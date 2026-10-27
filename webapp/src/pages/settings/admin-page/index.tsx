// The administration screen: libraries, scans, accounts, listeners.

import { useQuery } from "@tanstack/react-query";
import { type FormEvent, Fragment, useState } from "react";

import {
  addLibrary,
  createUser,
  currentUser,
  setUserDisabled,
  startScan,
  type User,
} from "../../../api";
import { administrationQuery } from "../../../api/queries";
import {
  Loading,
  PageHeader,
  useReload,
} from "../../../components/layout/page-shell";
import { useI18n } from "../../../state/i18n";
import { NowPlayingPanel, ScanProgress } from "./activity";
import { ApiTokensPanel, CredentialForm } from "./credentials";
import { LibraryMembersPanel, LibrarySettingsPanel } from "./libraries";

export function AdminPage() {
  const signedInUser = currentUser();
  const { t } = useI18n();
  const reload = useReload(administrationQuery().queryKey);
  const [notice, setNotice] = useState<string | null>(null);
  const [adminError, setAdminError] = useState<string | null>(null);
  const [libraryName, setLibraryName] = useState("");
  const [libraryPath, setLibraryPath] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [libraryBusy, setLibraryBusy] = useState(false);
  const [userBusy, setUserBusy] = useState(false);
  // The scan whose progress is on screen — whichever was started last from
  // here. Starting another replaces it rather than stacking panels.
  const [watchedScan, setWatchedScan] = useState<string | null>(null);
  const { data: value, error } = useQuery(administrationQuery());
  if (!value) return <Loading error={error} />;
  const [libraries, users] = value;

  async function registerLibrary(event: FormEvent) {
    event.preventDefault();
    setLibraryBusy(true);
    setAdminError(null);
    setNotice(null);
    try {
      const result = await addLibrary(libraryName, libraryPath, "private");
      setLibraryName("");
      setLibraryPath("");
      setNotice(t("admin.initialScan", { id: result.scan_id }));
      setWatchedScan(result.scan_id);
      reload();
    } catch {
      setAdminError(t("admin.libraryError"));
    } finally {
      setLibraryBusy(false);
    }
  }

  async function addUser(event: FormEvent) {
    event.preventDefault();
    setUserBusy(true);
    setAdminError(null);
    setNotice(null);
    try {
      await createUser(username, password, "user");
      setUsername("");
      setPassword("");
      reload();
    } catch {
      setAdminError(t("admin.accountError"));
    } finally {
      setUserBusy(false);
    }
  }

  async function toggleUser(user: User) {
    setAdminError(null);
    setNotice(null);
    try {
      await setUserDisabled(user.username, !user.disabled);
      reload();
    } catch {
      setAdminError(t("admin.statusError"));
    }
  }

  async function scanLibrary(libraryId: string) {
    setAdminError(null);
    setNotice(null);
    try {
      const result = await startScan(libraryId);
      setNotice(t("admin.scanQueued", { id: result.scan_id }));
      setWatchedScan(result.scan_id);
    } catch {
      setAdminError(t("admin.scanError"));
    }
  }

  return (
    <section className="admin-page">
      <PageHeader
        kicker={t("nav.settings")}
        title={t("admin.title")}
        detail={t("admin.detail")}
      />
      {notice ? <p className="notice">{notice}</p> : null}
      {adminError ? <p className="error">{adminError}</p> : null}
      {watchedScan ? (
        <article className="admin-panel scan-panel">
          <h3>{t("admin.scanning")}</h3>
          <ScanProgress scanId={watchedScan} />
        </article>
      ) : null}
      <div className="admin-grid">
        <article className="admin-panel">
          <h3>{t("admin.libraries")}</h3>
          <form
            className="stacked-form"
            onSubmit={(event) => void registerLibrary(event)}
          >
            <input
              value={libraryName}
              onChange={(event) => setLibraryName(event.target.value)}
              placeholder={t("admin.libraryName")}
              required
            />
            <input
              value={libraryPath}
              onChange={(event) => setLibraryPath(event.target.value)}
              placeholder={t("admin.libraryPath")}
              required
            />
            <button type="submit" disabled={libraryBusy}>
              {libraryBusy ? t("admin.registering") : t("admin.register")}
            </button>
          </form>
          <ul className="resource-list compact">
            {/* One pass, so each library's membership sits under that library.
                Two passes listed every library and then every panel, which put
                the third library's members four rows below it. */}
            {libraries.map((library) => (
              <Fragment key={library.id}>
                <li>
                  <div>
                    <strong>{library.name}</strong>
                    <span className="muted">{library.visibility}</span>
                  </div>
                  <button
                    type="button"
                    onClick={() => void scanLibrary(library.id)}
                  >
                    {t("admin.scan")}
                  </button>
                </li>
                <li className="member-row">
                  <LibraryMembersPanel libraryId={library.id} users={users} />
                </li>
                <li className="member-row">
                  <LibrarySettingsPanel
                    library={library}
                    onScan={setWatchedScan}
                    onDeleted={(message) => {
                      setAdminError(null);
                      setNotice(message);
                    }}
                  />
                </li>
              </Fragment>
            ))}
          </ul>
        </article>
        <article className="admin-panel">
          <h3>{t("admin.accounts")}</h3>
          <form
            className="stacked-form"
            onSubmit={(event) => void addUser(event)}
          >
            <input
              value={username}
              onChange={(event) => setUsername(event.target.value)}
              placeholder={t("login.username")}
              autoComplete="off"
              required
            />
            <input
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              placeholder={t("admin.webPassword")}
              minLength={12}
              autoComplete="new-password"
              required
            />
            <button type="submit" disabled={userBusy}>
              {userBusy ? t("admin.creating") : t("admin.create")}
            </button>
          </form>
        </article>
      </div>
      <div className="stack">
        {users.map((user) => (
          <article className="user-row" key={user.id}>
            <header>
              <div>
                <strong>{user.username}</strong>
                <span className="muted">
                  {user.role} · {user.folder_ids.length} libraries
                </span>
              </div>
              <button
                type="button"
                onClick={() => void toggleUser(user)}
                disabled={user.id === signedInUser?.id}
                title={
                  user.id === signedInUser?.id
                    ? t("admin.currentAccount")
                    : undefined
                }
              >
                {user.disabled ? t("admin.enable") : t("admin.disable")}
              </button>
            </header>
            <CredentialForm user={user} />
            <ApiTokensPanel username={user.username} />
          </article>
        ))}
      </div>
      <NowPlayingPanel />
    </section>
  );
}
