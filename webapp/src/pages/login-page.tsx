import { useNavigate } from "@tanstack/react-router";
import { type FormEvent, useEffect, useState } from "react";

import { bootstrapAdmin, login, safeInternalPath, setupRequired } from "../api";
import { useI18n } from "../state/i18n";

export function LoginPage() {
  const navigate = useNavigate();
  const { t } = useI18n();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
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
          await bootstrapAdmin(username, password);
          setSetup(false);
        } catch {
          setError(t("login.setupError"));
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
    </main>
  );
}
