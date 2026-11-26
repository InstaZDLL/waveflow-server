// A track's music video (RFC-029): in the immersive view, and from its menu.

import { expect, type Page, test } from "@playwright/test";

import { expectNoWcagViolations } from "./fixtures/accessibility";
import { resetAndMockApi } from "./fixtures/api";
import { song } from "./fixtures/catalogue";
import { silentWav } from "./fixtures/media";

test.beforeEach(resetAndMockApi);

/** What the server says of a track that has a clip (RFC-029 decision 8). */
const withClip = {
  ...song(1, "Hidden Place", 0, false),
  clip: { format: "mp4", byte_size: 1_234_567 },
};

/**
 * The clip's ticket and its bytes. A WAV, which a `<video>` plays for real,
 * so the element has a position and a pause to report, as a clip would.
 */
async function serveClips(page: Page) {
  await page.route("**/api/v2/tracks/*/clip-ticket", (route) => {
    const trackId = new URL(route.request().url()).pathname.split("/")[4];
    return route.fulfill({
      json: {
        url: `/api/v2/clip-stream/ticket-${trackId}`,
        expires_at: Date.now() + 3_600_000,
      },
    });
  });
  await page.route("**/api/v2/clip-stream/*", (route) =>
    route.fulfill({ contentType: "audio/wav", body: silentWav(4) }),
  );
}

/**
 * The clip is the playing screen's, not a view's. The immersive view is laid
 * over the detail view without unmounting it, so a clip the detail view held
 * kept sounding behind the immersive one; now exactly one view shows it, and
 * it follows the listener from one to the other.
 */
test("the clip follows from the detail view into the immersive one", async ({
  page,
}) => {
  await page.route("**/api/v2/queue", (route) =>
    route.fulfill({
      json: {
        current: "song-1",
        position_ms: 0,
        changed_by: null,
        updated_at: 1,
        songs: [withClip, song(2, "Cocoon", 0, false)],
      },
    }),
  );
  await serveClips(page);
  await page.goto("/playing");

  await page.getByRole("button", { name: "Watch the clip" }).click();
  const videos = page.locator("video");
  await expect(page.locator(".playing-track video")).toHaveAttribute(
    "src",
    "/api/v2/clip-stream/ticket-song-1",
  );

  await page.getByRole("link", { name: "Immersive" }).click();
  const immersive = page.locator("dialog.immersive");
  await expect(immersive.locator("video")).toBeVisible();
  // One picture, one sound: none left behind under the dialog.
  await expect(videos).toHaveCount(1);
  await expectNoWcagViolations(page);

  // Leaving the view hands the clip back to the detail view, still open.
  await page.keyboard.press("Escape");
  await expect(immersive).toHaveCount(0);
  await expect(page.locator(".playing-track video")).toBeVisible();
  await expect(videos).toHaveCount(1);

  await page.getByRole("button", { name: "Back to the track" }).click();
  await expect(videos).toHaveCount(0);

  // And it opens from the immersive view itself, in the cover's place.
  await page.getByRole("link", { name: "Immersive" }).click();
  await expect(immersive.locator(".immersive-cover")).toBeVisible();
  await immersive.getByRole("button", { name: "Watch the clip" }).click();
  await expect(immersive.locator("video")).toBeVisible();
  await expect(immersive.locator(".immersive-cover")).toHaveCount(0);
  await immersive.getByRole("button", { name: "Back to the track" }).click();
  await expect(immersive.locator(".immersive-cover")).toBeVisible();
  await expect(videos).toHaveCount(0);
});

/**
 * Any track's clip from its menu, in a dialog of its own: offered only where
 * the server says there is one, and Escape gives focus back to the row.
 */
test("a track's menu opens its clip in a dialog", async ({ page }) => {
  await page.route("**/api/v2/songs?*", (route) =>
    route.fulfill({ json: [withClip, song(2, "Cocoon", 0, false)] }),
  );
  await serveClips(page);
  await page.goto("/songs");

  const rows = page.locator(".songs tbody tr");
  const title = rows.nth(0).locator("button.song-title");
  await title.focus();
  await page.keyboard.press("Shift+F10");
  const menu = page.getByRole("menu", { name: "Actions for Hidden Place" });
  await expect(menu.getByRole("menuitem").nth(2)).toHaveText("Watch the clip");
  await menu.getByRole("menuitem", { name: "Watch the clip" }).click();

  const dialog = page.getByRole("dialog", { name: "Music video" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("Hidden Place")).toBeVisible();
  await expect(dialog.locator("video")).toHaveAttribute(
    "src",
    "/api/v2/clip-stream/ticket-song-1",
  );
  await expectNoWcagViolations(page);

  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(page.locator("video")).toHaveCount(0);
  await expect(title).toBeFocused();

  // A track the server gives no clip has no such entry.
  await rows.nth(1).locator("button.song-title").focus();
  await page.keyboard.press("Shift+F10");
  const other = page.getByRole("menu", { name: "Actions for Cocoon" });
  await expect(other).toBeVisible();
  await expect(
    other.getByRole("menuitem", { name: "Watch the clip" }),
  ).toHaveCount(0);
});
