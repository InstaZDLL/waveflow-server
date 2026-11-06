import { call } from "./client";

/** A channel manifest, as the release publishes it on R2 (RFC-028). */
export type UpdateManifest = {
  version: string;
  tag: string;
  published_at: string;
  /** The release's page: its notes and its downloads. An http(s) address. */
  page: string;
  notes: string | null;
};

/**
 * The update check as an administrator sees it.
 *
 * `decision` is the stored answer while `policy` is `ask`, and `null` until
 * someone answers: that is when the client puts the question. `latest` is set
 * only while the check is on.
 */
export type UpdateStatus = {
  policy: "ask" | "on" | "off";
  decision: boolean | null;
  enabled: boolean;
  current: string;
  channel: "stable" | "beta";
  latest: UpdateManifest | null;
  update_available: boolean;
  checked_at: number | null;
  failure: "unreachable" | "status" | "unreadable" | "invalid" | null;
};

const UPDATE = "/api/v2/server/update";

/** Administrators only; anyone else is refused. */
export const getUpdateStatus = () => call<UpdateStatus>(UPDATE);

/** An administrator's answer, under `ask`; `409` when the environment decided. */
export const setUpdateCheck = (enabled: boolean) =>
  call<UpdateStatus>(UPDATE, {
    method: "PUT",
    body: JSON.stringify({ enabled }),
  });
