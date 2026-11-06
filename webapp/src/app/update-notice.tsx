// The update dialog an administrator sees over the shell (RFC-028 decision 4):
// the question while nobody has answered it, then a newer version once one is
// known.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useId, useRef, useState } from "react";
import { currentUser, setUpdateCheck } from "../api";
import { updateStatusQuery } from "../api/queries";
import {
  dismissVersion,
  readDismissedVersion,
  updateNotice,
} from "../lib/update-notice";
import { useI18n } from "../state/i18n";

/**
 * Mounted once in the shell. Asks the route only for an administrator, so
 * nobody else sends the request or could be shown the dialog; the route would
 * refuse them anyway.
 */
export function UpdateNotice() {
  const admin = currentUser()?.role === "admin";
  const { data } = useQuery({ ...updateStatusQuery(), enabled: admin });
  // Read once: a dismissal in another tab does not reopen or close this one.
  const [dismissed, setDismissed] = useState(readDismissedVersion);
  // Closed with Later for this page's life; the next visit asks again.
  const [closed, setClosed] = useState(false);
  const notice = admin && !closed ? updateNotice(data, dismissed) : null;
  if (!notice) return null;
  return (
    <UpdateDialog
      notice={notice}
      current={data?.current ?? ""}
      onClose={() => setClosed(true)}
      onDismiss={(version) => {
        dismissVersion(version);
        setDismissed(version);
      }}
    />
  );
}

type DialogProps = {
  notice: NonNullable<ReturnType<typeof updateNotice>>;
  current: string;
  onClose: () => void;
  onDismiss: (version: string) => void;
};

/**
 * A native `<dialog>` opened with `showModal`, as the folder picker is: focus
 * is trapped and returned by the browser, and Escape closes it, which counts
 * as Later.
 */
function UpdateDialog({ notice, current, onClose, onDismiss }: DialogProps) {
  const { t } = useI18n();
  const queryClient = useQueryClient();
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    const node = dialog.current;
    if (node && !node.open) node.showModal();
  }, []);

  async function answer(enabled: boolean) {
    setBusy(true);
    setFailed(false);
    try {
      const status = await setUpdateCheck(enabled);
      queryClient.setQueryData(updateStatusQuery().queryKey, status);
      // A yes starts a check on the server; ask again once it has had time
      // to read the manifest, so a version already out shows this session.
      if (enabled) {
        window.setTimeout(() => {
          void queryClient.invalidateQueries({
            queryKey: updateStatusQuery().queryKey,
          });
        }, 15_000);
      }
    } catch {
      setFailed(true);
    } finally {
      setBusy(false);
    }
  }

  return (
    <dialog
      ref={dialog}
      className="update-notice"
      aria-labelledby={titleId}
      onClose={onClose}
    >
      {notice.kind === "ask" ? (
        <>
          <h2 id={titleId}>{t("update.askTitle")}</h2>
          <p>{t("update.askBody")}</p>
          {failed ? (
            <p className="error" role="alert">
              {t("update.saveError")}
            </p>
          ) : null}
          <div className="update-actions">
            <button
              type="button"
              disabled={busy}
              onClick={() => void answer(false)}
            >
              {t("update.askNo")}
            </button>
            <button
              type="button"
              className="action-primary"
              disabled={busy}
              onClick={() => void answer(true)}
            >
              {t("update.askYes")}
            </button>
          </div>
        </>
      ) : (
        <>
          <h2 id={titleId}>{t("update.title", { version: notice.version })}</h2>
          <p className="muted">{t("update.current", { current })}</p>
          <p>{t("update.howTo")}</p>
          <p>
            <a href={notice.page} target="_blank" rel="noreferrer">
              {t("update.notes")}
            </a>
          </p>
          <div className="update-actions">
            <button
              type="button"
              onClick={() => {
                onDismiss(notice.version);
                dialog.current?.close();
              }}
            >
              {t("update.dismiss")}
            </button>
            <button
              type="button"
              className="action-primary"
              onClick={() => dialog.current?.close()}
            >
              {t("update.later")}
            </button>
          </div>
        </>
      )}
    </dialog>
  );
}
