import { call } from "./client";

/**
 * What an account wants for an app that does not ask for a format or a bit rate
 * (RFC-027): a default, and any number of rows for one Subsonic client name.
 *
 * `format` is one of the status route's `formats` or, on a client's row, `raw`,
 * which keeps that client on the original. `bitrate` is the ceiling in kbit/s,
 * `null` exactly on `raw`.
 */
export type TranscodingProfile = {
  /** The client name this row is for; `null` on the default. */
  client: string | null;
  format: string;
  bitrate: number | null;
  updated_at: number;
};

export type TranscodingProfiles = {
  /** The default first, then the clients by name. */
  profiles: TranscodingProfile[];
  /**
   * The names this account's clients have streamed with without asking, since
   * the server started: the clients a profile reaches.
   */
  seen_clients: string[];
};

/** A row, whole: saving one for the same client replaces it. */
export type TranscodingProfileDraft = {
  client?: string;
  format: string;
  /** Left out, the format's default. */
  bitrate?: number;
};

const PROFILES = "/api/v2/transcoding-profiles";

export const getTranscodingProfiles = () => call<TranscodingProfiles>(PROFILES);

export const saveTranscodingProfile = (draft: TranscodingProfileDraft) =>
  call<TranscodingProfile>(PROFILES, {
    method: "PUT",
    body: JSON.stringify(draft),
  });

/** Removes a client's row, or the default with `null`. */
export const deleteTranscodingProfile = (client: string | null) =>
  call<void>(
    client === null
      ? PROFILES
      : `${PROFILES}?client=${encodeURIComponent(client)}`,
    { method: "DELETE" },
  );
