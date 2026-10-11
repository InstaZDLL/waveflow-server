import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { type FormEvent, useState } from "react";

import { createOwnApiToken, revokeOwnApiToken } from "./api";
import { LanguagePicker, type Locale, useI18n } from "./i18n";
import { Loading, PageHeader, useReload } from "./page-shell";
import { ThemePicker } from "./preferences";
import { ownApiTokensQuery } from "./queries";

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
    </section>
  );
}
