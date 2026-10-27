import { useState } from "react";

import { authorize, isAllowedRedirect } from "../../api";
import { useI18n } from "../../state/i18n";

/**
 * Consent screen for the native Authorization Code + PKCE flow.
 *
 * The desktop application opens this URL in the system browser with its PKCE
 * parameters; approving posts them back with the browser session attached and
 * follows the redirect the server computes.
 */
export function AuthorizePage() {
  const { t } = useI18n();
  const params = new URLSearchParams(window.location.search);
  const clientId = params.get("client_id") ?? "";
  const redirectUri = params.get("redirect_uri") ?? "";
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // Refuse the whole screen when the target is one we would not navigate to:
  // failing here is clearer than offering a Cancel button that cannot honour
  // its own redirect.
  const missing =
    !clientId ||
    !redirectUri ||
    !params.get("code_challenge") ||
    !isAllowedRedirect(redirectUri);

  async function approve() {
    setBusy(true);
    setError(null);
    try {
      const response = await authorize({
        client_id: clientId,
        redirect_uri: redirectUri,
        code_challenge: params.get("code_challenge") ?? "",
        code_challenge_method: params.get("code_challenge_method") ?? "S256",
        state: params.get("state"),
        device_name: params.get("device_name") ?? clientId,
      });
      window.location.assign(response.redirect_to);
    } catch {
      setError(t("authorize.error"));
      setBusy(false);
    }
  }

  function deny() {
    if (!isAllowedRedirect(redirectUri)) return;
    const url = new URL(redirectUri);
    url.searchParams.set("error", "access_denied");
    const state = params.get("state");
    if (state) url.searchParams.set("state", state);
    window.location.assign(url.toString());
  }

  if (missing) {
    return (
      <section>
        <h2>{t("authorize.title")}</h2>
        <p className="error">{t("authorize.invalid")}</p>
      </section>
    );
  }

  return (
    <section className="consent">
      <h2>{t("authorize.client", { client: clientId })}</h2>
      <p className="muted">{t("authorize.detail")}</p>
      <p className="muted">{t("authorize.redirect", { url: redirectUri })}</p>
      {error && <p className="error">{error}</p>}
      <div className="consent-actions">
        <button type="button" onClick={() => void approve()} disabled={busy}>
          {busy ? t("authorize.busy") : t("authorize.approve")}
        </button>
        <button type="button" onClick={deny} disabled={busy}>
          {t("common.cancel")}
        </button>
      </div>
    </section>
  );
}
