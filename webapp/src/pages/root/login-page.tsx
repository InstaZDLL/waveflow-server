import { Link, useNavigate } from "@tanstack/react-router";
import { type FormEvent, useEffect, useState } from "react";

import {
  ApiError,
  bootstrapAdmin,
  login,
  SETUP_TOKEN_REJECTED,
  safeInternalPath,
  setupRequired,
} from "../../api";
import { SourceOffer } from "../../components/layout/source-offer";
import { keptSongs, offlineSupported } from "../../lib/offline";
import { useI18n } from "../../state/i18n";

/**
 * A way to what this browser kept for offline listening (RFC-022), shown only
 * when it kept something: signed out, or with no server to sign in to, the
 * music already here still plays.
 */
function OfflineLink() {
  const { t } = useI18n();
  const [count, setCount] = useState(0);
  useEffect(() => {
    if (!offlineSupported()) return;
    let live = true;
    void keptSongs()
      .then((songs) => live && setCount(songs.length))
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, []);
  if (count === 0) return null;
  return (
    <p className="login-offline">
      <Link to="/offline">{t("login.offline", { count })}</Link>
    </p>
  );
}

export function LoginPage() {
  const navigate = useNavigate();
  const { t } = useI18n();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [setupToken, setSetupToken] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [setup, setSetup] = useState(false);

  useEffect(() => {
    void setupRequired().then(setSetup, () => setSetup(false));
  }, []);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      if (setup) {
        try {
          await bootstrapAdmin(username, password, setupToken);
          setSetup(false);
          setSetupToken("");
        } catch (failure) {
          setError(
            failure instanceof ApiError &&
              failure.message === SETUP_TOKEN_REJECTED
              ? t("login.setupTokenError")
              : t("login.setupError"),
          );
          return;
        }
      }
      try {
        await login(username, password);
      } catch {
        setError(t("login.error"));
        return;
      }
      const next = safeInternalPath(
        sessionStorage.getItem("waveflow.after-login"),
      );
      sessionStorage.removeItem("waveflow.after-login");
      if (next) window.location.assign(next);
      else await navigate({ to: "/" });
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="centered">
      <div className="login-shell">
        <section className="login-story" aria-hidden="true">
          <span className="eyebrow">{t("login.heroEyebrow")}</span>
          <h1>{t("login.heroTitle")}</h1>
          <div className="signal" />
          <p>{t("login.heroDetail")}</p>
        </section>
        <form className="card login-card" onSubmit={submit}>
          <span className="eyebrow">{t("login.server")}</span>
          <h1>{setup ? t("login.createAdmin") : t("login.welcome")}</h1>
          {setup ? <p className="muted">{t("login.setupDetail")}</p> : null}
          {setup ? (
            <label>
              {t("login.setupToken")}
              <input
                value={setupToken}
                onChange={(event) => setSetupToken(event.target.value)}
                autoComplete="off"
                spellCheck={false}
                aria-describedby="setup-token-hint"
                required
              />
              <span id="setup-token-hint" className="muted">
                {t("login.setupTokenHint")}
              </span>
            </label>
          ) : null}
          <label>
            {t("login.username")}
            <input
              value={username}
              onChange={(event) => setUsername(event.target.value)}
              autoComplete="username"
              required
            />
          </label>
          <label>
            {t("login.password")}
            <input
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              autoComplete={setup ? "new-password" : "current-password"}
              minLength={setup ? 12 : undefined}
              required
            />
          </label>
          {error && <p className="error">{error}</p>}
          <button type="submit" disabled={busy}>
            {busy
              ? t("login.wait")
              : setup
                ? t("login.create")
                : t("login.signIn")}
          </button>
        </form>
      </div>
      <OfflineLink />
      <SourceOffer />
    </main>
  );
}
