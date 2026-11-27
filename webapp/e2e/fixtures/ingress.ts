// Home Assistant's ingress, as far as the client can tell (RFC-030).

import { type Page, test } from "@playwright/test";

/** The prefix Home Assistant serves an app under, one token per installation. */
export const PREFIX = "/api/hassio_ingress/e2e_tok";

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
 * the source check in `src/api/base.test.ts` cannot see. It sees only what the
 * tests that use it make the page ask for.
 */
export async function serveUnderPrefix(page: Page): Promise<string[]> {
  const strayed: string[] = [];
  const origin = new URL(
    test.info().project.use.baseURL ?? "http://127.0.0.1:4173",
  ).origin;
  await page.route(`${origin}/**`, async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname !== PREFIX && !url.pathname.startsWith(`${PREFIX}/`)) {
      strayed.push(url.pathname);
      await route.abort();
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
  return strayed;
}
