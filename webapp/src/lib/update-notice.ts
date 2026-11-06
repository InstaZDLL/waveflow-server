// What the update dialog shows an administrator, if anything (RFC-028
// decision 4). No React here, so the rules are tested on their own.

import type { UpdateStatus } from "../api/server-update";

/** The two things the dialog can say, or nothing. */
export type UpdateNotice =
  | { kind: "ask" }
  | { kind: "update"; version: string; page: string }
  | null;

/**
 * The question first, while nobody has answered it: no version can be known
 * before the check is on. Then a newer version, unless this browser was told
 * to stop showing that one.
 */
export function updateNotice(
  status: UpdateStatus | undefined,
  dismissed: string | null,
): UpdateNotice {
  if (!status) return null;
  if (status.policy === "ask" && status.decision === null) {
    return { kind: "ask" };
  }
  if (!status.update_available || !status.latest) return null;
  if (dismissed === status.latest.version) return null;
  return {
    kind: "update",
    version: status.latest.version,
    page: status.latest.page,
  };
}

const DISMISSED_KEY = "waveflow.update.dismissed";

/**
 * The version this browser was told to stop announcing. Per version, so the
 * next release asks again; per browser, because it is a convenience and not a
 * decision the server has to keep. Storage that throws — a private window,
 * blocked site data — reads as nothing dismissed.
 */
export function readDismissedVersion(): string | null {
  try {
    return window.localStorage.getItem(DISMISSED_KEY);
  } catch {
    return null;
  }
}

export function dismissVersion(version: string): void {
  try {
    window.localStorage.setItem(DISMISSED_KEY, version);
  } catch {
    // Not kept: the dialog comes back on the next visit, which is harmless.
  }
}
