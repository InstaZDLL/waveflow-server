// The web client inside Home Assistant, under its ingress prefix (RFC-030).

import { expect, test } from "@playwright/test";

import { resetAndMockApi } from "./fixtures/api";
import { PREFIX, serveUnderPrefix } from "./fixtures/ingress";
import { silentWav } from "./fixtures/media";

test.beforeEach(resetAndMockApi);

/**
 * The paths the client reaches the server by, all under the prefix: the
 * shell's own files, the API, the artwork, a ticket the server answers as a
 * root path, the router's own addresses and the logo's image. One request at
 * the root and the test fails, whatever built its address.
 */
test("the client plays and moves about under Home Assistant's prefix", async ({
  page,
}) => {
  const streamed: string[] = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (path.includes("/api/v2/stream/")) streamed.push(path);
  });
  await page.route("**/api/v2/tracks/*/stream-ticket", (route) =>
    route.fulfill({
      json: {
        // A root path, as the server answers it.
        url: "/api/v2/stream/ticket-e2e",
        expires_at: Date.now() + 60_000,
      },
    }),
  );
  const audio = silentWav(60);
  await page.route("**/api/v2/stream/*", (route) =>
    route.fulfill({ body: audio, headers: { "content-type": "audio/wav" } }),
  );
  const refused = await serveUnderPrefix(page);

  await page.goto(`${PREFIX}/albums/album-2`);
  await page.getByRole("button", { name: "Play album" }).click();
  await expect(page.locator(".player-track strong")).toHaveText("Hidden Place");
  // The ticket's root path, asked for under the prefix.
  await expect.poll(() => streamed.length).toBeGreaterThan(0);
  for (const path of streamed) expect(path.startsWith(`${PREFIX}/`)).toBe(true);

  // The router's own links stay under the prefix.
  await page.getByRole("link", { name: "Songs" }).first().click();
  await expect(page).toHaveURL(new RegExp(`${PREFIX}/songs$`));
  await expect(page.locator(".songs tbody tr").first()).toBeVisible();

  expect(refused).toEqual({ strayed: [], withoutSession: [] });
});

/**
 * RFC-030 decision 8: with no public address, the server refuses to make a
 * share whose link nobody outside the panel could open, and the page says why
 * instead of offering the form again.
 */
test("a share inside Home Assistant needs the server's public address", async ({
  page,
}) => {
  await page.route("**/api/v2/shares", (route) =>
    route.request().method() === "POST"
      ? route.fulfill({
          status: 409,
          json: {
            code: "share_address_missing",
            message:
              "A share link needs the server's external address, WAVEFLOW_PUBLIC_URL",
          },
        })
      : route.fallback(),
  );
  const refused = await serveUnderPrefix(page);

  // The queue the server keeps comes back with the page, so there is one to
  // share.
  await page.goto(`${PREFIX}/settings/shares`);
  await expect(
    page.getByRole("button", { name: "Share current queue" }),
  ).toBeEnabled();
  await page.getByLabel("Description").fill("For a friend");
  await page.getByRole("button", { name: "Share current queue" }).click();
  await expect(page.getByRole("status")).toContainText("WAVEFLOW_PUBLIC_URL");
  await expect(
    page.getByRole("button", { name: "Share current queue" }),
  ).toHaveCount(0);
  expect(refused).toEqual({ strayed: [], withoutSession: [] });
});

/**
 * Last.fm sends the listener back to the server's own address, whose cookies
 * the panel never had: the page says where to link it from instead of
 * starting a journey that cannot finish.
 */
test("Last.fm is linked from the server's own address", async ({ page }) => {
  const refused = await serveUnderPrefix(page);
  await page.goto(`${PREFIX}/settings/scrobbling`);
  await expect(
    page.getByText("connect it from there rather than from Home Assistant"),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Connect to Last.fm" }),
  ).toHaveCount(0);
  expect(refused).toEqual({ strayed: [], withoutSession: [] });
});

/**
 * The probe is a public request, sent without this page's cookies at the
 * root. Under the prefix it has to carry Home Assistant's `ingress_session`,
 * or Home Assistant refuses it and the source offer is never drawn.
 */
test("the probe is asked for with Home Assistant's session", async ({
  page,
}) => {
  const refused = await serveUnderPrefix(page);
  await page.goto(`${PREFIX}/settings`);
  await expect(page.getByRole("link", { name: "Source code" })).toBeVisible();
  expect(refused).toEqual({ strayed: [], withoutSession: [] });
});

/**
 * The fixture's own premise, held on every browser project the suite runs:
 * the `Cookie` header of an intercepted request is visible to it. A request
 * that carries Home Assistant's session goes through, and the same request
 * sent without it is refused. Were the header not visible, the first would be
 * refused too, and every other test here would mean nothing.
 */
test("the fixture tells a request with Home Assistant's session from one without", async ({
  page,
}) => {
  const refused = await serveUnderPrefix(page);
  const shell = await page.goto(`${PREFIX}/settings`);
  expect(shell?.status()).toBe(200);
  const statuses = await page.evaluate(async (prefix) => {
    const carried = await fetch(`${prefix}/health`);
    const omitted = await fetch(`${prefix}/health`, { credentials: "omit" });
    return [carried.status, omitted.status];
  }, PREFIX);
  expect(statuses).toEqual([200, 401]);
  expect(refused.withoutSession).toEqual([`${PREFIX}/health`]);
  expect(refused.strayed).toEqual([]);
});
