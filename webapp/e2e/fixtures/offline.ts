// Keeping a track for offline, and reading back what the browser kept.

import { expect, type Page } from "@playwright/test";

import { mock } from "./state";

/** Keeps the fixture's track for offline through its menu, as a listener does. */
export async function keepFirstSongOffline(page: Page) {
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
  await page
    .getByRole("menu", { name: "Actions for Army Of Me" })
    .getByRole("menuitem", { name: "Keep for offline" })
    .click();
  await expect.poll(() => keptPaths(page)).toContain("/offline/tracks/song-9");
}

/** Every address this browser holds in an offline cache, whoever kept it. */
export function keptPaths(page: Page) {
  return page.evaluate(async () => {
    const paths: string[] = [];
    for (const name of await caches.keys()) {
      if (!name.startsWith("waveflow-offline-v1:")) continue;
      const cache = await caches.open(name);
      for (const request of await cache.keys()) {
        paths.push(new URL(request.url).pathname);
      }
    }
    return paths.sort();
  });
}
