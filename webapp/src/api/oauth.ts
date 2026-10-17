import { call } from "./client";

export type AuthorizeRequest = {
  client_id: string;
  redirect_uri: string;
  code_challenge: string;
  code_challenge_method: string;
  state: string | null;
  device_name: string;
};

export const authorize = (request: AuthorizeRequest) =>
  call<{ redirect_to: string }>("/api/v2/oauth/authorize", {
    method: "POST",
    body: JSON.stringify(request),
  });

/**
 * Mirrors the server's `validate_redirect_uri` policy.
 *
 * The consent screen navigates to the client's redirect target itself when the
 * user cancels, and that target arrives in the query string. Without this check
 * the page is an open redirect: no authorisation code leaks, but a crafted link
 * borrows the server's origin to bounce a visitor anywhere. The server remains
 * the authority for approvals; this guards the navigation the client performs.
 */
export function isAllowedRedirect(redirectUri: string): boolean {
  let url: URL;
  try {
    url = new URL(redirectUri);
  } catch {
    return false;
  }
  if (url.hash) return false;
  switch (url.protocol) {
    case "http:":
      return ["127.0.0.1", "[::1]", "localhost"].includes(url.hostname);
    case "https:":
      return true;
    default:
      // A private-use scheme must be a reverse-domain name, never a bare word
      // another application could plausibly claim.
      return url.protocol.slice(0, -1).includes(".");
  }
}

/**
 * Narrows a remembered destination to a same-document path.
 *
 * The value is read back from storage and handed to `location.assign`, which
 * resolves far more than a path: `//host` is protocol-relative and leaves the
 * origin entirely, and a `javascript:` value would execute. Only a single
 * leading slash followed by a non-slash, non-backslash character is a path —
 * browsers normalise backslashes, so `/\host` escapes just like `//host`.
 */
export function safeInternalPath(value: string | null): string | null {
  if (!value || value.length < 1) return null;
  if (value[0] !== "/") return null;
  if (value[1] === "/" || value[1] === "\\") return null;
  return value;
}
