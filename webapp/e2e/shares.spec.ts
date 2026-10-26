// Share links: the page a share opens, what it plays and its covers.

import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

import { resetAndMockApi } from "./fixtures/api";
import { song } from "./fixtures/catalogue";

test.beforeEach(resetAndMockApi);

/**
 * A share link opens a player for somebody with no account (RFC-020
 * decision 3). The server hands a browser this page; the page asks the same
 * address for the share, and plays from the addresses it is given.
 */
test("a share link opens a player that plays what the share hands out", async ({
  page,
}) => {
  const shareAsks: Array<string | null> = [];
  await page.route("**/share/wfs_e2e", async (route) => {
    // The navigation itself is the page, which `vite preview` serves.
    if (route.request().resourceType() === "document") {
      await route.fallback();
      return;
    }
    shareAsks.push(await route.request().headerValue("authorization"));
    await route.fulfill({
      json: {
        id: "share-1",
        description: "For the drive",
        expiresAt: null,
        visitCount: 1,
        tracks: [
          {
            ...song(1, "Hidden Place", 5, true),
            streamUrl: "/share/wfs_e2e/tracks/song-1/stream?visit=pass-1",
          },
          {
            ...song(1, "Hidden Place", 5, true),
            streamUrl: "/share/wfs_e2e/tracks/song-1/stream?visit=pass-2",
          },
        ],
      },
    });
  });
  await page.route("**/share/wfs_e2e/tracks/**", (route) =>
    route.fulfill({ status: 204, body: "" }),
  );
  await page.goto("/share/wfs_e2e");
  await expect(
    page.getByRole("heading", { name: "For the drive" }),
  ).toBeVisible();
  // No rail and no sign-in: the visitor has neither.
  await expect(page.getByRole("navigation")).toHaveCount(0);
  // Named by what the row shows, which axe holds a label to.
  const plays = page.getByRole("button", { name: /^Hidden Place/ });
  await expect(plays).toHaveCount(2);
  await plays.last().click();
  await expect(page.getByLabel("Player")).toHaveAttribute(
    "src",
    /\/share\/wfs_e2e\/tracks\/song-1\/stream\?visit=pass-2/,
  );
  // Asked once, and without the session a signed-in visitor holds: the share
  // route reads none, and every ask is a visit.
  expect(shareAsks).toEqual([null]);
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});

test("a share link that opens nothing says so", async ({ page }) => {
  await page.route("**/share/wfs_gone", async (route) => {
    if (route.request().resourceType() === "document") {
      await route.fallback();
      return;
    }
    await route.fulfill({ status: 404, body: "" });
  });
  await page.goto("/share/wfs_gone");
  await expect(
    page.getByRole("heading", { name: "This link opens nothing" }),
  ).toBeVisible();
});

/**
 * A share page shows the covers the share hands out, from the addresses it
 * hands out (RFC-020): plain image addresses under the visit's pass, loaded
 * with no session. A track with no cover shows a plain square.
 */
test("a share page shows the covers its share hands out", async ({ page }) => {
  const coverAsks: Array<string | null> = [];
  await page.route("**/share/wfs_covers", async (route) => {
    if (route.request().resourceType() === "document") {
      await route.fallback();
      return;
    }
    await route.fulfill({
      json: {
        id: "share-2",
        description: "Covers",
        expiresAt: null,
        visitCount: 1,
        tracks: [
          {
            ...song(1, "Hidden Place", 5, true),
            streamUrl: "/share/wfs_covers/tracks/song-1/stream?visit=pass-1",
            coverUrl: "/share/wfs_covers/tracks/song-1/cover?visit=pass-1",
          },
          {
            ...song(2, "Cocoon", 0, false),
            streamUrl: "/share/wfs_covers/tracks/song-2/stream?visit=pass-2",
          },
        ],
      },
    });
  });
  await page.route("**/share/wfs_covers/tracks/*/cover?*", async (route) => {
    coverAsks.push(await route.request().headerValue("authorization"));
    await route.fulfill({
      status: 200,
      headers: { "content-type": "image/png" },
      body: Buffer.from(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z4m8AAAAASUVORK5CYII=",
        "base64",
      ),
    });
  });
  await page.goto("/share/wfs_covers");
  await expect(page.getByRole("heading", { name: "Covers" })).toBeVisible();
  const covers = page.locator("img.share-track-cover");
  await expect(covers).toHaveCount(1);
  await expect(covers.first()).toHaveAttribute(
    "src",
    "/share/wfs_covers/tracks/song-1/cover?visit=pass-1",
  );
  await expect.poll(() => coverAsks).toEqual([null]);
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});

/** A queue longer than a share holds is not sent, and the page says why. */
test("does not send a queue longer than a share holds", async ({ page }) => {
  const long = Array.from({ length: 401 }, (_, index) =>
    song(index + 1, `Track ${index + 1}`, 0, false),
  );
  await page.route("**/api/v2/queue", (route) =>
    route.fulfill({
      json: {
        current: "song-1",
        position_ms: 0,
        changed_by: null,
        updated_at: 1,
        songs: long,
      },
    }),
  );
  const created: string[] = [];
  await page.route("**/api/v2/shares", async (route) => {
    if (route.request().method() === "POST") created.push(route.request().url());
    await route.fulfill({ json: [] });
  });
  await page.goto("/shares");
  await expect(
    page.getByText(
      "A share holds up to 400 tracks, and the queue holds 401. Shorten the queue to share it.",
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Share current queue" }),
  ).toBeDisabled();
  expect(created).toEqual([]);
});

/** A cover that will not load gives way to the plain square. */
test("a share page falls back to a plain square when a cover will not load", async ({
  page,
}) => {
  await page.route("**/share/wfs_broken", async (route) => {
    if (route.request().resourceType() === "document") {
      await route.fallback();
      return;
    }
    await route.fulfill({
      json: {
        id: "share-3",
        description: "Broken cover",
        expiresAt: null,
        visitCount: 1,
        tracks: [
          {
            ...song(1, "Hidden Place", 5, true),
            streamUrl: "/share/wfs_broken/tracks/song-1/stream?visit=pass-1",
            coverUrl: "/share/wfs_broken/tracks/song-1/cover?visit=pass-1",
          },
        ],
      },
    });
  });
  await page.route("**/share/wfs_broken/tracks/*/cover?*", (route) =>
    route.fulfill({ status: 404, body: "" }),
  );
  await page.goto("/share/wfs_broken");
  await expect(page.getByRole("heading", { name: "Broken cover" })).toBeVisible();
  await expect(page.locator("img.share-track-cover")).toHaveCount(0);
  await expect(page.locator("span.share-track-cover")).toHaveCount(1);
});
