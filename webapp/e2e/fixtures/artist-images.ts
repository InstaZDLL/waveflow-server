// An artist's banner and portrait, served and changed, and their menu.

import { expect, type Page } from "@playwright/test";

import { artistDetail } from "./catalogue";

/**
 * Opens the artist header's image menu, where the portrait and banner actions
 * live behind one button, and returns the menu.
 */
export async function openImageMenu(page: Page) {
  await page
    .locator(".artist-hero")
    .getByRole("button", { name: "Edit images", exact: true })
    .click();
  const menu = page.getByRole("menu", { name: "Images of Björk" });
  await expect(menu).toBeVisible();
  return menu;
}

/**
 * A stand-in for the artist routes of RFC-013 with state: the page re-reads
 * after every write, and what it shows must be what the write did.
 */
export async function serveArtistImages(page: Page) {
  const state = {
    portrait: null as string | null,
    banner: null as string | null,
    writes: [] as Array<{ method: string; kind: string; bytes: number }>,
    refuseNext: false,
  };
  const detail = () => ({
    ...artistDetail,
    library_id: "library-1",
    artwork_hash: state.portrait ?? "f".repeat(64),
    ...(state.banner ? { banner_hash: state.banner } : {}),
    ...(state.portrait || state.banner
      ? {
          image_choice: {
            ...(state.portrait
              ? { portrait: { source: "upload", set_at: 1 } }
              : {}),
            ...(state.banner
              ? { banner: { source: "upload", set_at: 1 } }
              : {}),
          },
        }
      : {}),
  });
  await page.route("**/api/v2/artists/artist-1", (route) =>
    route.fulfill({ json: detail() }),
  );
  await page.route("**/api/v2/artists/artist-1/image/*", async (route) => {
    const request = route.request();
    const kind = new URL(request.url()).pathname.split("/").at(-1) ?? "";
    const bytes = request.postDataBuffer()?.length ?? 0;
    state.writes.push({ method: request.method(), kind, bytes });
    if (request.method() === "DELETE") {
      if (kind === "portrait") state.portrait = null;
      else state.banner = null;
      await route.fulfill({ status: 204, body: "" });
      return;
    }
    if (state.refuseNext) {
      state.refuseNext = false;
      await route.fulfill({ status: 422, json: { error: "validation" } });
      return;
    }
    const hash = (kind === "portrait" ? "b" : "c").repeat(64);
    if (kind === "portrait") state.portrait = hash;
    else state.banner = hash;
    await route.fulfill({ json: detail() });
  });
  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
    "base64",
  );
  await page.route("**/api/v2/artwork/*", (route) =>
    route.fulfill({ contentType: "image/png", body: png }),
  );
  return { state, png };
}
