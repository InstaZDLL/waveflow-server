import { ApiError } from "./client";

/** The running server as its liveness probe describes it. */
export type ServerIdentity = {
  version: string;
  /**
   * The source of this version — the repository at the tag `v<version>` —
   * which the server offers to everyone using it (AGPL-3.0 §13).
   */
  source: string;
};

/**
 * What the server says it is, and where its source is.
 *
 * Read from `/health` rather than built here from a version: the server owns
 * the address, so a modified build that names its own repository in its
 * manifest moves this link with it, and the page cannot point at source other
 * than the one the server it talks to was built from.
 *
 * **Plain `fetch`, not the session client.** The probe is public and the
 * sign-in page asks before anyone has a session.
 */
export async function getServerIdentity(): Promise<ServerIdentity> {
  const response = await fetch("/health", {
    headers: { accept: "application/json" },
    credentials: "omit",
  });
  if (!response.ok) throw new ApiError(response.status, "GET /health");
  return (await response.json()) as ServerIdentity;
}
