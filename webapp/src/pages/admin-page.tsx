import { useQuery } from "@tanstack/react-query";
import { type FormEvent, Fragment, useEffect, useState } from "react";

import {
  ApiError,
  addLibrary,
  createApiToken,
  createUser,
  currentUser,
  deleteLibrary,
  type Library,
  removeLibraryMember,
  revokeApiToken,
  type ScanJob,
  setLibraryMember,
  setSubsonicCredential,
  setUserDisabled,
  startScan,
  type User,
  updateLibrary,
  watchScan,
} from "../api";
import {
  administrationQuery,
  apiTokensQuery,
  libraryMembersQuery,
  librarySettingsQuery,
  nowPlayingQuery,
} from "../api/queries";
import {
  Loading,
  PageHeader,
  useReload,
  Waiting,
} from "../components/page-shell";
import { type TranslationKey, useI18n } from "../state/i18n";

function CredentialForm({ user }: { user: User }) {
  const { t } = useI18n();
  const [password, setPassword] = useState("");
  const [apiKey, setApiKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  async function submit(event: FormEvent) {
    event.preventDefault();
    setError(null);
    try {
      const result = await setSubsonicCredential(user.username, password);
      setApiKey(result.api_key);
      setPassword("");
    } catch {
      setError(t("credential.error"));
    }
  }
  return (
    <form className="credential-form" onSubmit={(event) => void submit(event)}>
      <input
        type="password"
        value={password}
        onChange={(event) => setPassword(event.target.value)}
        placeholder={t("credential.password")}
        aria-label={`${t("credential.password")} — ${user.username}`}
        minLength={12}
        required
      />
      <button type="submit">{t("credential.rotate")}</button>
      {apiKey ? (
        <output className="secret-output">
          {t("credential.copy")} <code>{apiKey}</code>
        </output>
      ) : null}
      {error ? <span className="error">{error}</span> : null}
    </form>
  );
}

/**
 * The statuses `scan_job.status` holds. Looked up rather than cast: an unknown
 * key makes `translate` read a property of `undefined` and takes the page down,
 * so a status this table does not know is shown as it came.
 */
const SCAN_STATUS: Record<string, TranslationKey> = {
  queued: "scan.queued",
  running: "scan.running",
  completed: "scan.completed",
  failed: "scan.failed",
};

/**
 * A scan while it runs. The admin screen used to start one and say nothing
 * more; the progress stream has been there all along.
 */
function ScanProgress({ scanId }: { scanId: string }) {
  const { t } = useI18n();
  const [job, setJob] = useState<ScanJob | null>(null);
  const [lost, setLost] = useState(false);

  useEffect(() => {
    setJob(null);
    setLost(false);
    // The snapshot arrives on the stream itself, so there is nothing to fetch
    // first; a stream that cannot open leaves the panel on its last reading.
    return watchScan(scanId, setJob, () => setLost(true));
  }, [scanId]);

  // A stream that cannot open has to say so: without this the panel waited on
  // a first reading that was never coming, for as long as the page stayed up.
  if (!job && lost) {
    return (
      <p className="error" role="alert">
        {t("scan.lost")}
      </p>
    );
  }
  if (!job) return <p className="muted">{t("scan.connecting")}</p>;
  const done = job.status !== "running" && job.status !== "queued";
  const share = job.total_files > 0 ? job.processed_files / job.total_files : 0;
  return (
    <div className="scan-progress">
      <div className="scan-line">
        <strong>
          {SCAN_STATUS[job.status] ? t(SCAN_STATUS[job.status]) : job.status}
        </strong>
        <span className="muted">
          {t("scan.counted", {
            processed: job.processed_files,
            total: job.total_files,
          })}
        </span>
      </div>
      <progress
        max={job.total_files || 1}
        value={job.processed_files}
        aria-label={t("scan.progress")}
      >
        {Math.round(share * 100)}%
      </progress>
      <p className="muted scan-tally">
        {t("scan.tally", {
          added: job.added,
          updated: job.updated,
          moved: job.moved,
          skipped: job.skipped,
          errors: job.errors,
        })}
      </p>
      {/* What the walk left out (RFC-021): only when there is something to
          say, since most libraries have neither. */}
      {job.excluded > 0 || job.skipped_links > 0 ? (
        <p className="muted scan-tally">
          {t("scan.leftOut", {
            excluded: job.excluded,
            links: job.skipped_links,
          })}
        </p>
      ) : null}
      {/* The path is the only thing here that moves fast enough to read as
          liveness, and it is what tells a long scan from a stuck one. */}
      {!done && job.current_path ? (
        <p className="muted scan-path">{job.current_path}</p>
      ) : null}
      {job.message ? <p className="error">{job.message}</p> : null}
    </div>
  );
}

/** Who is listening to what, right now, across the accounts one can see. */
function NowPlayingPanel() {
  const { t } = useI18n();
  // No stream exists for this one, so it is polled, on the interval the query
  // carries. A query keeps the reading it has while it refetches, so the panel
  // no longer has to hold the previous one in a state of its own to avoid
  // flashing "nobody is listening" every thirty seconds.
  const { data: entries } = useQuery(nowPlayingQuery());

  return (
    <article className="admin-panel now-playing-panel">
      <h3>{t("admin.nowPlaying")}</h3>
      {entries?.length ? (
        <ul className="list resource-list">
          {entries.map((entry) => (
            <li key={`${entry.username}-${entry.song.id}`}>
              <div>
                <strong>{entry.song.title}</strong>
                <small className="muted">
                  {entry.song.artist ?? t("common.unknownArtist")}
                </small>
              </div>
              <span className="muted">{entry.username}</span>
            </li>
          ))}
        </ul>
      ) : (
        <p className="muted">{t("admin.nobodyListening")}</p>
      )}
    </article>
  );
}

/** API tokens for one account, so a client can be authorised without the CLI. */
function ApiTokensPanel({ username }: { username: string }) {
  const { t } = useI18n();
  const reload = useReload(apiTokensQuery(username, true).queryKey);
  const [name, setName] = useState("");
  const [secret, setSecret] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  // Closed until asked for. The panel is rendered once per account, so loading
  // on mount meant one request per account every time the admin screen opened
  // — for a list almost nobody opens.
  const [open, setOpen] = useState(false);
  const { data: value, error } = useQuery(apiTokensQuery(username, open));

  async function issue(event: FormEvent) {
    event.preventDefault();
    setFailed(false);
    try {
      const created = await createApiToken(username, name);
      setName("");
      // Shown once and never again: only its SHA-256 hash is stored.
      setSecret(created.secret);
      reload();
    } catch {
      setFailed(true);
    }
  }

  async function revoke(id: string) {
    setFailed(false);
    try {
      await revokeApiToken(username, id);
      reload();
    } catch {
      setFailed(true);
    }
  }

  const live = (value ?? []).filter((token) => token.revoked_at === null);
  return (
    // A disclosure button rather than `<details>`: `onToggle` is a non-bubbling
    // event that did not reach React here, so the panel opened on screen while
    // the state gating the request stayed false — and controlling `<details>`
    // by cancelling the summary's click takes its keyboard handling with it.
    <article className="admin-panel token-panel">
      <h3>
        <button
          type="button"
          aria-expanded={open}
          onClick={() => setOpen((value) => !value)}
        >
          {t("admin.tokens", { username })}
        </button>
      </h3>
      {open ? (
        <>
          <form className="inline-form" onSubmit={(event) => void issue(event)}>
            <input
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder={t("admin.tokenName")}
              aria-label={t("admin.tokenName")}
              required
            />
            <button type="submit">{t("admin.tokenCreate")}</button>
          </form>
          {failed ? <p className="error">{t("admin.tokenError")}</p> : null}
          {secret ? (
            <p className="notice">
              {t("admin.tokenOnce")}{" "}
              <code className="secret-output">{secret}</code>
            </p>
          ) : null}
          {live.length ? (
            <ul className="list resource-list">
              {live.map((token) => (
                <li key={token.id}>
                  <div>
                    <strong>{token.name}</strong>
                    <small className="muted">
                      {token.last_used_at
                        ? t("admin.tokenUsed")
                        : t("admin.tokenUnused")}
                    </small>
                  </div>
                  <button
                    type="button"
                    className="danger"
                    onClick={() => void revoke(token.id)}
                    aria-label={`${t("admin.tokenRevoke")}: ${token.name}`}
                  >
                    {t("admin.tokenRevoke")}
                  </button>
                </li>
              ))}
            </ul>
          ) : error ? (
            // A list that could not be fetched is not an empty list. Saying
            // "no token on this account" here would be a claim about the
            // account, made out of a failed request.
            <p className="error" role="alert">
              {t("common.loadError")}
            </p>
          ) : (
            <p className="muted">{t("admin.tokenNone")}</p>
          )}
        </>
      ) : null}
    </article>
  );
}

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
function LibraryMembersPanel({
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
function LibrarySettingsPanel({
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
