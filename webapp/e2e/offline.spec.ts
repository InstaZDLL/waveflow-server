// Tracks kept for offline listening, and what a sign-out does to them.

import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

import { resetAndMockApi } from "./fixtures/api";
import { session } from "./fixtures/catalogue";
import { keepFirstSongOffline, keptPaths } from "./fixtures/offline";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

/**
 * "Keep for offline" stores a track in this browser under its id, fetched
 * once through a ticket whose address is not what is kept (RFC-022), and the
 * offline page lists it and plays it from there with no server to ask.
 */
test("keeps a track for offline, by its id, and plays it from the offline page", async ({
  page,
}) => {
  await page.route("**/api/v2/songs?*", (route) =>
    route.fulfill({ json: [mock.correctable.song] }),
  );
  await page.route("**/api/v2/tracks/song-9/stream-ticket", (route) =>
    route.fulfill({
      json: { url: "/api/v2/stream/ticket-9", expires_at: Date.now() + 60_000 },
    }),
  );
  await page.route("**/api/v2/stream/ticket-9", (route) =>
    route.fulfill({
      status: 200,
      headers: { "content-type": "audio/flac" },
      body: Buffer.from([0x66, 0x4c, 0x61, 0x43]),
    }),
  );
  await page.goto("/songs");
  await page
    .locator(".songs tbody tr")
    .first()
    .locator("button.song-title")
    .focus();
  await page.keyboard.press("Shift+F10");
  const menu = page.getByRole("menu", { name: "Actions for Army Of Me" });
  await menu.getByRole("menuitem", { name: "Keep for offline" }).click();
  const stored = () =>
    page.evaluate(async () => {
      const cache = await caches.open("waveflow-offline-v1:user-1");
      return (await cache.keys())
        .map((request) => new URL(request.url).pathname)
        .sort();
    });
  await expect
    .poll(stored)
    .toEqual(["/offline/meta/song-9", "/offline/tracks/song-9"]);
  expect((await stored()).some((path) => path.includes("ticket"))).toBe(false);

  // The offline page asks the server for nothing.
  const asked: string[] = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (path.startsWith("/api/")) asked.push(path);
  });
  await page.goto("/offline");
  await expect(
    page.getByRole("heading", { name: "Kept for offline" }),
  ).toBeVisible();
  await expect(page.getByRole("navigation")).toHaveCount(0);
  await page.getByRole("button", { name: /^Army Of Me/ }).click();
  await expect(page.getByLabel("Offline player")).toHaveAttribute(
    "src",
    /^(blob:|\/offline\/tracks\/song-9$)/,
  );
  expect(asked).toEqual([]);
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);

  await page
    .getByRole("button", { name: "Remove the offline copy of Army Of Me" })
    .click();
  await expect(
    page.getByText("Nothing is kept yet.", { exact: false }),
  ).toBeVisible();
  await expect.poll(stored).toEqual([]);
});

/**
 * The kept tracks are one account's (RFC-022): a sign-out deletes them, so
 * the offline page opened afterwards — with no session at all — lists and
 * plays nothing of the account that left.
 */
test("a sign-out deletes the tracks kept for offline", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "signs out from the sidebar");
  await keepFirstSongOffline(page);
  // Dispatched rather than clicked: the sidebar's footer sits under the
  // fixed player bar at some viewports.
  await page.getByRole("button", { name: "Sign out" }).dispatchEvent("click");
  await expect(page).toHaveURL(/\/login$/);
  await expect.poll(() => keptPaths(page)).toEqual([]);

  await page.route("**/api/v2/web/auth/refresh", (route) =>
    route.fulfill({ status: 401, json: { code: "invalid_refresh_token" } }),
  );
  await page.goto("/offline");
  await expect(page.getByText(/Nothing is kept yet/)).toBeVisible();
  await expect(page.getByRole("button", { name: /^Army Of Me/ })).toHaveCount(
    0,
  );
  await expect(page.getByLabel("Offline player")).toHaveCount(0);
});

/**
 * Another account signing in on the same browser is shown none of the last
 * one's copies, and they are gone before it could ask for them.
 */
test("another account signing in sees none of the tracks kept offline", async ({
  page,
}) => {
  await keepFirstSongOffline(page);
  await page.route("**/api/v2/web/auth/refresh", (route) =>
    route.fulfill({
      json: {
        ...session,
        user: { id: "user-2", username: "guest", role: "user" },
      },
    }),
  );
  await page.goto("/songs");
  await expect.poll(() => keptPaths(page)).toEqual([]);
  await page.goto("/offline");
  await expect(page.getByText(/Nothing is kept yet/)).toBeVisible();
  await expect(page.getByRole("button", { name: /^Army Of Me/ })).toHaveCount(
    0,
  );
});

/**
 * The offline page plays a track by its id: forgetting a track listed above
 * it neither switches what plays nor starts it over (review of #335).
 */
test("keeps playing the same kept track when another is forgotten", async ({
  page,
}) => {
  await page.goto("/offline");
  await page.evaluate(async () => {
    await (await caches.open("waveflow-offline-owner")).put(
      "/offline/owner",
      new Response("user-1"),
    );
    const cache = await caches.open("waveflow-offline-v1:user-1");
    for (const [id, title, at] of [
      ["k1", "First kept", 2],
      ["k2", "Second kept", 1],
    ] as const) {
      await cache.put(
        `/offline/tracks/${id}`,
        new Response(new Blob([new Uint8Array([1, 2, 3])]), {
          headers: { "content-type": "audio/flac" },
        }),
      );
      await cache.put(
        `/offline/meta/${id}`,
        new Response(
          JSON.stringify({
            id,
            title,
            artist: "Ada Reed",
            album: null,
            duration_ms: 1_000,
            suffix: "flac",
            kept_at: at,
            bytes: 3,
          }),
        ),
      );
    }
  });
  await page.reload();
  await page.getByRole("button", { name: /^Second kept/ }).click();
  const player = page.getByLabel("Offline player");
  await expect(player).toHaveAttribute("src", /^(blob:|\/offline\/)/);
  const before = await player.getAttribute("src");
  await page
    .getByRole("button", { name: "Remove the offline copy of First kept" })
    .click();
  await expect(page.getByRole("button", { name: /^First kept/ })).toHaveCount(
    0,
  );
  await expect(
    page.getByRole("listitem").filter({ hasText: "Second kept" }),
  ).toHaveAttribute("aria-current", "true");
  await expect(player).toHaveAttribute("src", before ?? "");
});
