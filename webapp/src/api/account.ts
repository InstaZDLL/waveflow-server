import { call } from "./client";

export type ApiToken = {
  id: string;
  name: string;
  scopes: string[];
  expires_at: number | null;
  created_at: number;
  last_used_at: number | null;
  revoked_at: number | null;
};

/**
 * The clients this account has registered, newest first.
 *
 * Self-scoped, where the three calls above name a username and need an
 * administrator. A token is never a promotion — the server checks the
 * account's role apart from the credential's scopes — so who may see and
 * revoke an account's own clients is a question about that account.
 */
export const listOwnApiTokens = () => call<ApiToken[]>("/api/v2/api-tokens");

/**
 * Registers a client for this account. The secret comes back once and is
 * never recoverable: only its hash is kept.
 *
 * No scopes asked for, which means "everything this account has" — and that
 * is refused with 403 when the caller is itself a narrowed token, because a
 * credential cannot mint one wider than itself. From a signed-in session,
 * which is unrestricted, it is the ordinary case.
 */
export const createOwnApiToken = (name: string) =>
  call<ApiToken & { secret: string }>("/api/v2/api-tokens", {
    method: "POST",
    body: JSON.stringify({ name, scopes: [] }),
  });

/** Revoke one of this account's own clients. */
export const revokeOwnApiToken = (tokenId: string) =>
  call<void>(`/api/v2/api-tokens/${tokenId}`, { method: "DELETE" });

/**
 * Whether this account holds a dedicated Subsonic credential, and since when.
 *
 * `null` when it holds none. Never the password — the server's query selects
 * two timestamps and does not read the columns a secret lives in, so there is
 * nothing here to display by accident.
 */
export type SubsonicCredential = {
  created_at: number;
  updated_at: number;
};

export const readOwnSubsonicCredential = () =>
  call<SubsonicCredential | null>("/api/v2/subsonic-credential");

/**
 * Replaces this account's Subsonic password and API key, returning the new key
 * once.
 *
 * **Rotation, never creation.** An account holding none answers 404, because
 * an account without a credential cannot reach the Subsonic facade at all —
 * creating one opens a door rather than narrowing one already open, and that
 * is the operator's decision. So the panel offers this only where the read
 * above found something.
 *
 * Refused with 403 to a scoped or expiring credential: the key this returns
 * carries neither restriction, since the facade reads neither, so it cannot be
 * narrowed to match its caller the way an API token is. From a signed-in
 * session it is the ordinary case.
 */
export const rotateOwnSubsonicCredential = (password: string) =>
  call<{ api_key: string }>("/api/v2/subsonic-credential", {
    method: "PUT",
    body: JSON.stringify({ password }),
  });

/** Switches this account's Subsonic credential off. */
export const revokeOwnSubsonicCredential = () =>
  call<void>("/api/v2/subsonic-credential", { method: "DELETE" });
