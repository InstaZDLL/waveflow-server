import type { ApiToken } from "./account";
import { call } from "./client";

export type User = {
  id: string;
  username: string;
  role: "admin" | "user";
  disabled: boolean;
  has_subsonic_credential: boolean;
  folder_ids: string[];
};

export const listApiTokens = (username: string) =>
  call<ApiToken[]>(
    `/api/v2/admin/users/${encodeURIComponent(username)}/tokens`,
  );

/** The secret comes back once and is never recoverable: only its hash is kept. */
export const createApiToken = (username: string, name: string) =>
  call<ApiToken & { secret: string }>(
    `/api/v2/admin/users/${encodeURIComponent(username)}/tokens`,
    { method: "POST", body: JSON.stringify({ name, scopes: [] }) },
  );

/** Revoke an API token belonging to an account. */
export const revokeApiToken = (username: string, tokenId: string) =>
  call<void>(
    `/api/v2/admin/users/${encodeURIComponent(username)}/tokens/${tokenId}`,
    { method: "DELETE" },
  );

export const listUsers = () => call<User[]>("/api/v2/admin/users");

export const createUser = (
  username: string,
  webPassword: string,
  role: "admin" | "user",
) =>
  call<User>("/api/v2/admin/users", {
    method: "POST",
    body: JSON.stringify({ username, web_password: webPassword, role }),
  });

export const setUserDisabled = (username: string, disabled: boolean) =>
  call<User>(`/api/v2/admin/users/${encodeURIComponent(username)}`, {
    method: "PATCH",
    body: JSON.stringify({ disabled }),
  });

export const setSubsonicCredential = (username: string, password: string) =>
  call<{ api_key: string }>(
    `/api/v2/admin/users/${encodeURIComponent(username)}/subsonic-credential`,
    { method: "PUT", body: JSON.stringify({ password }) },
  );
