// One account's credentials: its Subsonic password and its API tokens.

import { useQuery } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";

import {
  createApiToken,
  revokeApiToken,
  setSubsonicCredential,
  type User,
} from "../../api";
import { apiTokensQuery } from "../../api/queries";
import { useReload } from "../../components/page-shell";
import { useI18n } from "../../state/i18n";

export function CredentialForm({ user }: { user: User }) {
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

/** API tokens for one account, so a client can be authorised without the CLI. */
export function ApiTokensPanel({ username }: { username: string }) {
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
