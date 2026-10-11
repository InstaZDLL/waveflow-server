import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { type FormEvent, useState } from "react";

import {
  createOwnApiToken,
  revokeOwnApiToken,
  revokeOwnSubsonicCredential,
  rotateOwnSubsonicCredential,
} from "./api";
import { LanguagePicker, type Locale, useI18n } from "./i18n";
import { Loading, PageHeader, useReload } from "./page-shell";
import { ThemePicker } from "./preferences";
import { ownApiTokensQuery, ownSubsonicCredentialQuery } from "./queries";

/**
 * Settings, and the clients allowed to read the library.
 *
 * The theme and the language used to sit in the sidebar footer, under thirteen
 * navigation entries — settings left in a corridor, because they are not
 * destinations. They are here now and **only** here: leaving a copy behind
 * would give two ways to ask one question, and the point of this page was that
 * the corridor was crowded.
 *
 * The clients are the part that did not exist anywhere. A credential on an
 * account could be listed only by the instance administrator, so a listener
 * could not find out which devices were reading their library, let alone stop
 * one. `/api/v2/api-tokens` answers that for the account asking.
 */

/** A timestamp as the reader writes dates, not as the wire carries them. */
function when(milliseconds: number, locale: Locale): string {
  return new Intl.DateTimeFormat(locale, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(milliseconds));
}

/**
 * The clients holding a credential on this account.
 *
 * Revoked and expired ones are left out rather than shown struck through:
 * this list answers "who can read my library", and a credential that cannot
 * is not an answer to it. The administration screen keeps the full history.
 */
function RegisteredClients() {
  const { locale, t } = useI18n();
  const reload = useReload(ownApiTokensQuery().queryKey);
  const [name, setName] = useState("");
  const [secret, setSecret] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const { data: value, error } = useQuery(ownApiTokensQuery());

  async function register(event: FormEvent) {
    event.preventDefault();
    setFailed(false);
    try {
      const created = await createOwnApiToken(name);
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
      await revokeOwnApiToken(id);
      reload();
    } catch {
      setFailed(true);
    }
  }

  // **Expired as well as revoked.** This list answers who *can* read the
  // library, and a credential past its expiry cannot — the lookup on the
  // server refuses it, so showing it here would name a device that has
  // already stopped. Revoked ones were always left out; expiry only became
  // reachable when a self-minted credential started carrying one, and the
  // reason to leave it out is the same reason.
  const now = Date.now();
  const live = (value ?? []).filter(
    (token) =>
      token.revoked_at === null &&
      (typeof token.expires_at !== "number" || token.expires_at > now),
  );

  return (
    <article className="settings-panel">
      <h3>{t("clients.title")}</h3>
      <p className="muted">{t("clients.detail")}</p>
      <form className="inline-form" onSubmit={(event) => void register(event)}>
        <input
          value={name}
          onChange={(event) => setName(event.target.value)}
          placeholder={t("clients.name")}
          aria-label={t("clients.name")}
          required
        />
        <button type="submit">{t("clients.register")}</button>
      </form>
      {failed ? <p className="error">{t("clients.error")}</p> : null}
      {secret ? (
        <p className="notice">
          {t("clients.once")} <code className="secret-output">{secret}</code>
        </p>
      ) : null}
      {error ? (
        // A list that could not be fetched is not an empty list. Saying "no
        // client holds a credential here" out of a failed request would be a
        // claim about the account.
        <p className="error" role="alert">
          {t("common.loadError")}
        </p>
      ) : !value ? (
        <Loading error={null} />
      ) : live.length ? (
        <ul className="list resource-list">
          {live.map((token) => (
            <li key={token.id}>
              <div>
                <strong>{token.name}</strong>
                <small className="muted">
                  {t("clients.registered", {
                    when: when(token.created_at, locale),
                  })}
                  {" · "}
                  {token.last_used_at === null
                    ? t("clients.unused")
                    : t("clients.used", {
                        when: when(token.last_used_at, locale),
                      })}
                </small>
              </div>
              <button
                type="button"
                className="danger"
                onClick={() => void revoke(token.id)}
                aria-label={`${t("clients.revoke")}: ${token.name}`}
              >
                {t("clients.revoke")}
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="muted">{t("clients.none")}</p>
      )}
    </article>
  );
}

/**
 * The separate password Subsonic clients sign in with.
 *
 * **Two of the three verbs, and the missing one is deliberate.** This panel
 * changes a credential or switches it off; it cannot create one. An account
 * without a Subsonic password cannot reach the facade at all, so creating one
 * opens a door rather than narrowing one already open — the operator's
 * decision, not a button here. Where there is nothing to rotate, the panel
 * says so and offers nothing, rather than showing a control that answers 404.
 *
 * Why it exists: that password is typed into third-party clients, on phones,
 * which is the ordinary way an application password leaks. Until now its owner
 * could neither change it nor stop it — both server routes were administrative
 * — so an account could not answer "my key has leaked" without finding an
 * operator.
 */
function SubsonicPassword() {
  const { locale, t } = useI18n();
  const reload = useReload(ownSubsonicCredentialQuery().queryKey);
  const [password, setPassword] = useState("");
  const [apiKey, setApiKey] = useState<string | null>(null);
  const [failed, setFailed] = useState<"error" | "short" | null>(null);
  const { data: value, error } = useQuery(ownSubsonicCredentialQuery());

  async function rotate(event: FormEvent) {
    event.preventDefault();
    // Said here rather than by `minLength`, and not as well as it: the
    // native rule would block the submit before this ran, so the message
    // below would be unreachable. The browser would also phrase its own
    // refusal in the browser's language rather than the application's, which
    // is the mistake the scrobbling `unavailable` cases were rewritten to
    // stop making.
    if (password.length < 12) {
      setFailed("short");
      return;
    }
    setFailed(null);
    try {
      const rotated = await rotateOwnSubsonicCredential(password);
      setPassword("");
      // Shown once and never again: only its SHA-256 hash is stored.
      setApiKey(rotated.api_key);
      reload();
    } catch {
      setFailed("error");
    }
  }

  async function switchOff() {
    setFailed(null);
    try {
      await revokeOwnSubsonicCredential();
      setApiKey(null);
      reload();
    } catch {
      setFailed("error");
    }
  }

  return (
    <article className="settings-panel">
      <h3>{t("subsonic.title")}</h3>
      <p className="muted">{t("subsonic.detail")}</p>
      {error ? (
        // A failed request is not an answer about the account. Saying "you
        // have no Subsonic password" out of one would be a claim.
        <p className="error" role="alert">
          {t("common.loadError")}
        </p>
      ) : value === undefined ? (
        // **`undefined` is "not loaded yet" and `null` is "holds none".**
        // They are two different things and the panel says different things
        // about them, so the test is on `undefined` and not on falsiness —
        // `!value` would call a loaded answer a pending one.
        <Loading error={null} />
      ) : value === null ? (
        <p className="muted">{t("subsonic.none")}</p>
      ) : (
        <>
          <p className="muted">
            <small>
              {t("subsonic.set", { when: when(value.created_at, locale) })}
              {value.updated_at > value.created_at ? (
                <>
                  {" · "}
                  {t("subsonic.changed", {
                    when: when(value.updated_at, locale),
                  })}
                </>
              ) : null}
            </small>
          </p>
          <form
            className="inline-form"
            onSubmit={(event) => void rotate(event)}
          >
            <input
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              placeholder={t("subsonic.password")}
              aria-label={t("subsonic.password")}
              autoComplete="new-password"
              required
            />
            <button type="submit">{t("subsonic.rotate")}</button>
          </form>
          <p className="muted">
            <small>{t("subsonic.gone")}</small>
          </p>
          <button
            type="button"
            className="danger"
            onClick={() => void switchOff()}
          >
            {t("subsonic.revoke")}
          </button>
        </>
      )}
      {failed ? (
        <p className="error">
          {failed === "short" ? t("subsonic.short") : t("subsonic.error")}
        </p>
      ) : null}
      {apiKey ? (
        <p className="notice">
          {t("subsonic.once")} <code className="secret-output">{apiKey}</code>
        </p>
      ) : null}
    </article>
  );
}

export function SettingsPage() {
  const { t } = useI18n();
  return (
    <section>
      <PageHeader title={t("nav.settings")} detail={t("settings.detail")} />
      <div className="settings-grid">
        <article className="settings-panel">
          <h3>{t("settings.appearance")}</h3>
          <ThemePicker />
          <LanguagePicker />
        </article>
        <article className="settings-panel">
          <h3>{t("nav.scrobbling")}</h3>
          <p className="muted">{t("settings.scrobblingDetail")}</p>
          <Link to="/settings/scrobbling">{t("settings.scrobblingOpen")}</Link>
        </article>
      </div>
      <RegisteredClients />
      <SubsonicPassword />
    </section>
  );
}
