// Settings › Administration › Updates: the switch of the update check, and
// what it last found (RFC-028).

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { setUpdateCheck } from "../../../api";
import {
  refreshUpdateStatusSoon,
  updateStatusQuery,
} from "../../../api/queries";
import { useI18n } from "../../../state/i18n";

export function UpdatesPanel() {
  const { t, locale } = useI18n();
  const queryClient = useQueryClient();
  const { data: status } = useQuery(updateStatusQuery());
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);

  if (!status) return null;

  async function turn(enabled: boolean) {
    setBusy(true);
    setFailed(false);
    try {
      const next = await setUpdateCheck(enabled);
      queryClient.setQueryData(updateStatusQuery().queryKey, next);
      if (enabled) refreshUpdateStatusSoon(queryClient);
    } catch {
      setFailed(true);
    } finally {
      setBusy(false);
    }
  }

  const when =
    status.checked_at === null
      ? null
      : new Intl.DateTimeFormat(locale, {
          dateStyle: "medium",
          timeStyle: "short",
        }).format(status.checked_at);

  return (
    <article className="admin-panel updates-panel">
      <h3>{t("update.panel")}</h3>
      <p>{t(status.enabled ? "update.enabled" : "update.disabled")}</p>
      {status.policy === "on" ? (
        <p className="muted">{t("update.policyOn")}</p>
      ) : null}
      {status.policy === "off" ? (
        <p className="muted">{t("update.policyOff")}</p>
      ) : null}
      {status.enabled ? (
        <>
          <p className="muted">
            {when ? t("update.lastChecked", { when }) : t("update.never")}
          </p>
          {status.failure ? (
            <p className="error">{t("update.failed")}</p>
          ) : null}
          {status.latest && status.update_available ? (
            <p>
              {t("update.available", { version: status.latest.version })}{" "}
              <a href={status.latest.page} target="_blank" rel="noreferrer">
                {t("update.notes")}
              </a>
            </p>
          ) : null}
          {status.latest && !status.update_available ? (
            <p className="muted">
              {t("update.upToDate", { current: status.current })}
            </p>
          ) : null}
        </>
      ) : null}
      {failed ? (
        <p className="error" role="alert">
          {t("update.saveError")}
        </p>
      ) : null}
      {status.policy === "ask" ? (
        <button
          type="button"
          className={
            status.enabled ? "update-toggle" : "update-toggle action-primary"
          }
          disabled={busy}
          onClick={() => void turn(!status.enabled)}
        >
          {t(status.enabled ? "update.turnOff" : "update.turnOn")}
        </button>
      ) : null}
    </article>
  );
}
