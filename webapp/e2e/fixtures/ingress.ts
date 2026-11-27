// Home Assistant's ingress, as far as the client can tell (RFC-030).

import { type Page, test } from "@playwright/test";

/** The prefix Home Assistant serves an app under, one token per installation. */
export const PREFIX = "/api/hassio_ingress/e2e_tok";

/** What the page sent that Home Assistant would not have let through. */
export type Refused = {
  /** Requests that reached the root instead of the prefix. */
  strayed: string[];
  /**
   * Requests under the prefix that came without Home Assistant's own
   * `ingress_session` cookie, which its ingress refuses before the server
   * sees them — a `fetch` with `credentials: "omit"`, for one.
   */
  withoutSession: string[];
};

/**
 * Serves the client under {@link PREFIX}, as Home Assistant does, and records
 * every request that reaches the root instead.
 *
 * Registered after the mocks of `api.ts`, so it is asked first. A request
 * under the prefix is handed on with the prefix stripped, as Home Assistant
 * hands it to the server: an API call to the mocks, which read the path they
 * always did; a file to the preview, the shell rewritten as the server writes
 * it under a prefix. A request outside the prefix is refused and kept, and a
 * test asserts there were none — whatever built its address, which is what
 * the source check in `src/api/base.test.ts` cannot see. A request under the
 * prefix without Home Assistant's `ingress_session` cookie is answered 401,
 * as Home Assistant answers it, and kept too. It sees only what the tests that
 * use it make the page ask for.
 */
export async function serveUnderPrefix(page: Page): Promise<Refused> {
  const refused: Refused = { strayed: [], withoutSession: [] };
  const origin = new URL(
    test.info().project.use.baseURL ?? "http://127.0.0.1:4173",
  ).origin;
  // Home Assistant's session for its ingress, scoped as it scopes it.
  await page.context().addCookies([
    {
      name: "ingress_session",
      value: "e2e-session",
      domain: new URL(origin).hostname,
      path: "/api/hassio_ingress/",
    },
  ]);
  await page.route(`${origin}/**`, async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname !== PREFIX && !url.pathname.startsWith(`${PREFIX}/`)) {
      refused.strayed.push(url.pathname);
      await route.abort();
      return;
    }
    const cookie = (await route.request().headerValue("cookie")) ?? "";
    if (!/(?:^|;s*)ingress_session=/.test(cookie)) {
      refused.withoutSession.push(url.pathname);
      await route.fulfill({ status: 401, body: "401: Unauthorized" });
      return;
    }
    const stripped = new URL(url);
    stripped.pathname = url.pathname.slice(PREFIX.length) || "/";
    const server =
      stripped.pathname.startsWith("/api/") ||
      stripped.pathname.startsWith("/share/") ||
      stripped.pathname === "/health";
    if (server) {
      await route.fallback({ url: stripped.toString() });
      return;
    }
    const response = await route.fetch({ url: stripped.toString() });
    if (!(response.headers()["content-type"] ?? "").includes("text/html")) {
      await route.fulfill({ response });
      return;
    }
    const html = (await response.text())
      .replace(/\b(src|href)="\/(?!\/)/g, `$1="${PREFIX}/`)
      .replace(
        'name="waveflow-base" content=""',
        `name="waveflow-base" content="${PREFIX}"`,
      )
      .replace(/<link rel="manifest"[^>]*>/, "");
    await route.fulfill({ response, body: html });
  });
  return refused;
}
