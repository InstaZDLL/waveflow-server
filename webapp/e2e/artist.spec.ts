// The artist page: most played, enrichment, discography and its images.

import { expect, test } from "@playwright/test";

import {
  chooseLightTheme,
  expectNoWcagViolations,
  lowestContrast,
} from "./fixtures/accessibility";
import { resetAndMockApi } from "./fixtures/api";
import { openImageMenu, serveArtistImages } from "./fixtures/artist-images";
import { albums, artistDetail, library } from "./fixtures/catalogue";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

/**
 * An artist page carries what this account played, and nothing when it played
 * nothing.
 *
 * The desktop client fills this part of an artist page from a remote service.
 * This server queries none, so what is counted is local listening — which also
 * means a fresh account sees no section at all, and that is the half worth
 * pinning: a heading over an empty table would read as a broken feature rather
 * than as an honest answer.
 */
test("an artist page shows this account's most played, and no heading without a history", async ({
  page,
}) => {
  await page.goto("/artists/artist-1");
  await expect(
    page.getByRole("heading", { name: "Björk", level: 2 }),
  ).toBeVisible();

  await expect(
    page.getByRole("heading", { name: "Most played here" }),
  ).toBeVisible();
  // The server ranked them; the client renders that order and does not re-sort.
  await expect(page.locator(".artist-played .songs tbody tr")).toHaveCount(2);
  await expect(
    page.locator(".artist-played .song-title").first(),
  ).toContainText("Cocoon");
  // A screenful, not the discography.
  expect(mock.topSongAsks).toEqual(["10"]);
  // And the albums are still what the page is.
  await expect(page.getByRole("link", { name: "Open the album: Vespertine" })).toBeVisible();

  await expectNoWcagViolations(page);

  // Nothing played: the section is absent, not empty.
  mock.artistPlayed = [];
  await page.goto("/artists/artist-1");
  await expect(
    page.getByRole("heading", { name: "Björk", level: 2 }),
  ).toBeVisible();
  await expect(page.getByRole("link", { name: "Open the album: Vespertine" })).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Most played here" }),
  ).toHaveCount(0);
});

/**
 * RFC-011 decision 11 on the screen: what a provider said, and which one said
 * it. The first answer is `pending`, as an artist's first visit is; the page
 * asks again on its own, since the web client reads no event stream, and the
 * second answer fills it.
 */
test("an artist page shows what the server learned, and where each part came from", async ({
  page,
}) => {
  const lastfm = { provider: "lastfm", fetched_at: Date.UTC(2026, 9, 14) };
  mock.enrichmentOf = (ask) =>
    ask === 1
      ? { state: "pending", bio: null, image: null, similar: null }
      : {
          state: "ready",
          bio: {
            summary: "An Icelandic singer and songwriter.",
            full: "An Icelandic singer and songwriter.\nHer albums are many.",
            source: lastfm,
          },
          image: {
            artwork_hash: "a".repeat(64),
            source: { provider: "deezer", fetched_at: Date.UTC(2026, 9, 15) },
          },
          similar: {
            artists: [
              {
                id: "artist-2",
                name: "Sigur Rós",
                album_count: 1,
                artwork_hash: null,
              },
            ],
            source: lastfm,
          },
        };
  // Only the portrait's hash answers with an image: the artist row names
  // none, so the hero can show it only through `enrichment.image`.
  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
    "base64",
  );
  await page.route(`**/api/v2/artwork/${"a".repeat(64)}`, async (route) => {
    await route.fulfill({ contentType: "image/png", body: png });
  });

  await page.goto("/artists/artist-1");
  await expect(page.getByRole("status")).toContainText(
    "Looking this artist up",
  );
  await expect(page.getByRole("heading", { name: "About" })).toBeVisible({
    timeout: 10_000,
  });
  await expect(page.getByText("An Icelandic singer and songwriter.").first()).toBeVisible();
  await expect(page.getByText(/^Biography from Last\.fm, /)).toBeVisible();
  await expect(page.getByText(/^Portrait from Deezer, /)).toBeVisible();
  await expect(page.locator(".artist-hero img.artist-portrait")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Similar artists in your library" }),
  ).toBeVisible();
  await expect(page.getByRole("link", { name: /Sigur Rós/ })).toBeVisible();
  await expect(page.getByText(/^Suggested by Last\.fm, /)).toBeVisible();
  await expect(page.getByText("Looking this artist up")).toHaveCount(0);
  expect(mock.artistAsks).toBe(2);

  await expectNoWcagViolations(page);
});

/** `none` is an answer: nothing is shown, and nothing is asked again. */
test("an artist nobody knows shows no enrichment and is not asked again", async ({
  page,
}) => {
  mock.enrichmentOf = () => ({ state: "none", bio: null, image: null, similar: null });
  await page.goto("/artists/artist-1");
  await expect(page.getByRole("link", { name: "Open the album: Vespertine" })).toBeVisible();
  await page.waitForTimeout(4_000);
  await expect(page.getByRole("heading", { name: "About" })).toHaveCount(0);
  await expect(page.getByText("Looking this artist up")).toHaveCount(0);
  expect(mock.artistAsks).toBe(1);
});

/**
 * A server that fails after a `pending` answer leaves that answer on screen,
 * and a failed ask does not move the count of answers. The failures are
 * counted with them, so the page still stops at the bound.
 */
test("a pending artist whose server keeps failing is asked a bounded number of times", async ({
  page,
}) => {
  test.setTimeout(45_000);
  mock.artistFailsAfterFirst = true;
  mock.enrichmentOf = () => ({ state: "pending", bio: null, image: null, similar: null });
  await page.goto("/artists/artist-1");
  await expect(page.getByRole("heading", { name: "Björk", level: 2 })).toBeVisible();
  await expect.poll(() => mock.artistAsks, { timeout: 30_000 }).toBe(6);
  await page.waitForTimeout(5_000);
  expect(mock.artistAsks).toBe(6);
  await expect(page.getByText("Looking this artist up")).toHaveCount(0);
});

/**
 * Choosing an artist's banner and portrait by hand (RFC-013 step 3): the file
 * goes as its own bytes, the header draws the banner behind it, a chosen
 * portrait says so, and each can go back to automatic.
 */
test("changes an artist's banner and portrait from the artist page", async ({
  page,
}) => {
  const { state, png } = await serveArtistImages(page);
  await page.goto("/artists/artist-1");
  const hero = page.locator(".artist-hero");
  // The actions sit behind one button in the header's corner.
  let menu = await openImageMenu(page);
  await expect(menu.getByRole("menuitem", { name: "Change portrait" })).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: "Change banner" })).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: "No banner" })).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);
  // With no banner, the portrait is the backdrop, blurred.
  await expect(hero.locator(".artist-hero-ambient")).toBeAttached();

  await hero.getByLabel("Change banner").setInputFiles({
    name: "banner.png",
    mimeType: "image/png",
    buffer: png,
  });
  await expect(hero.getByRole("status")).toHaveText("Banner changed.");
  await expect(hero.locator("img.artist-banner")).toBeVisible();
  await expect(hero.locator(".artist-hero-ambient")).toHaveCount(0);
  expect(state.writes).toEqual([{ method: "PUT", kind: "banner", bytes: png.length }]);
  await expectNoWcagViolations(page);

  await hero.getByLabel("Change portrait").setInputFiles({
    name: "portrait.png",
    mimeType: "image/png",
    buffer: png,
  });
  await expect(hero.getByRole("status")).toHaveText("Portrait changed.");
  await expect(hero.getByText("Portrait chosen by hand")).toBeVisible();

  // A file the server refuses is said, and nothing on screen changes.
  state.refuseNext = true;
  await hero.getByLabel("Change portrait").setInputFiles({
    name: "notes.png",
    mimeType: "image/png",
    buffer: Buffer.from("not an image"),
  });
  await expect(hero.getByRole("alert")).toHaveText("Choose a JPEG, PNG or WebP image.");

  // Back to automatic: no banner, and the fetched portrait again.
  menu = await openImageMenu(page);
  await menu.getByRole("menuitem", { name: "No banner" }).click();
  await expect(hero.getByRole("status")).toHaveText("Banner removed.");
  await expect(hero.locator("img.artist-banner")).toHaveCount(0);
  menu = await openImageMenu(page);
  await menu.getByRole("menuitem", { name: "Automatic portrait" }).click();
  await expect(hero.getByRole("status")).toHaveText("Back to the automatic portrait.");
  await expect(hero.getByText("Portrait chosen by hand")).toHaveCount(0);
  expect(state.writes.slice(-2)).toEqual([
    { method: "DELETE", kind: "banner", bytes: 0 },
    { method: "DELETE", kind: "portrait", bytes: 0 },
  ]);
});

/** A listener sees the chosen banner and is offered nothing to change. */
test("shows a listener the artist's banner with nothing to change", async ({
  page,
}) => {
  mock.libraries = [{ ...library("library-1", "Ma musique"), role: "listener" }];
  const { state } = await serveArtistImages(page);
  state.banner = "c".repeat(64);
  await page.goto("/artists/artist-1");
  const hero = page.locator(".artist-hero");
  await expect(hero.locator("img.artist-banner")).toBeVisible();
  // Not even the button the actions sit behind.
  await expect(hero.getByRole("button", { name: "Edit images" })).toHaveCount(0);
  await expect(hero.getByRole("button", { name: /Change/ })).toHaveCount(0);
  await expect(hero.getByRole("button", { name: "Search Deezer" })).toHaveCount(0);
});

/**
 * The automatic banner (RFC-014): TheAudioDB's fanart is drawn behind the
 * header and credited to TheAudioDB, whose terms ask for it; a banner chosen
 * by hand is credited to nobody.
 */
test("credits TheAudioDB for an automatic banner and nobody for a chosen one", async ({
  page,
}) => {
  await serveArtistImages(page);
  let chosen = false;
  // Cached but not yet linked on the artist row: the enrichment alone names
  // it, as it can on the visit after an artist is re-identified.
  let linked = false;
  await page.route("**/api/v2/artists/artist-1", (route) =>
    route.fulfill({
      json: {
        ...artistDetail,
        library_id: "library-1",
        ...(chosen || linked
          ? { banner_hash: (chosen ? "b" : "c").repeat(64) }
          : {}),
        ...(chosen
          ? { image_choice: { banner: { source: "upload", set_at: 1 } } }
          : {}),
        enrichment: {
          state: "ready",
          bio: null,
          image: null,
          similar: null,
          banner: chosen
            ? null
            : {
                artwork_hash: "c".repeat(64),
                source: {
                  provider: "theaudiodb",
                  fetched_at: Date.UTC(2026, 9, 24),
                },
              },
        },
      },
    }),
  );

  await page.goto("/artists/artist-1");
  const hero = page.locator(".artist-hero");
  await expect(hero.locator("img.artist-banner")).toBeVisible();
  await expect(hero.getByText(/^Banner from TheAudioDB, /)).toBeVisible();

  linked = true;
  await page.reload();
  await expect(hero.locator("img.artist-banner")).toBeVisible();
  await expect(hero.getByText(/^Banner from TheAudioDB, /)).toBeVisible();
  await expectNoWcagViolations(page);

  chosen = true;
  await page.reload();
  await expect(hero.locator("img.artist-banner")).toBeVisible();
  await expect(hero.getByText(/^Banner from /)).toHaveCount(0);
});

/**
 * Choosing the portrait from Deezer (RFC-013 step 4): the picker searches the
 * artist's name as it opens, shows previews the server holds and nothing from
 * Deezer itself, says what a refusal means, and a pick sends the Deezer id
 * alone.
 */
test("chooses an artist's portrait from a Deezer search", async ({ page }) => {
  const { state } = await serveArtistImages(page);
  const searches: string[] = [];
  const picks: unknown[] = [];
  const thirdParty: string[] = [];
  let refuseNext = false;
  page.on("request", (request) => {
    if (/deezer\.com|dzcdn\.net/.test(request.url())) thirdParty.push(request.url());
  });
  await page.route("**/api/v2/artists/artist-1/image/candidates?*", async (route) => {
    const query = new URL(route.request().url()).searchParams.get("q") ?? "";
    searches.push(query);
    if (refuseNext) {
      refuseNext = false;
      await route.fulfill({ status: 429, json: { code: "too_many_requests" } });
      return;
    }
    await route.fulfill({
      json:
        query === "Nobody"
          ? []
          : [
              { deezer_id: 76, name: "Björk", fans: 1500000, artwork_hash: "d".repeat(64) },
              { deezer_id: 77, name: "Björk Tribute", fans: 1, artwork_hash: "e".repeat(64) },
            ],
    });
  });
  await page.route("**/api/v2/artists/artist-1/image/portrait/deezer", async (route) => {
    picks.push(route.request().postDataJSON());
    state.portrait = "d".repeat(64);
    await route.fulfill({
      json: {
        ...artistDetail,
        library_id: "library-1",
        artwork_hash: state.portrait,
        image_choice: { portrait: { source: "deezer", set_at: 1 } },
      },
    });
  });

  await page.goto("/artists/artist-1");
  const hero = page.locator(".artist-hero");
  await (await openImageMenu(page))
    .getByRole("menuitem", { name: "Search Deezer" })
    .click();
  const picker = page.getByRole("dialog", { name: "Choose a portrait from Deezer" });
  await expect(picker).toBeVisible();
  const results = picker.getByRole("list", { name: "Deezer artists" });
  await expect(results.getByRole("button")).toHaveCount(2);
  await expect(results.getByText("1 fan", { exact: true })).toBeVisible();
  expect(searches).toEqual(["Björk"]);
  await expectNoWcagViolations(page);

  const query = picker.getByRole("searchbox", { name: "Artist name" });
  await query.fill("Nobody");
  await picker.getByRole("button", { name: "Search", exact: true }).click();
  await expect(picker.getByRole("status")).toHaveText(
    "Deezer knows no artist by that name.",
  );

  refuseNext = true;
  await query.fill("Bjork");
  await picker.getByRole("button", { name: "Search", exact: true }).click();
  await expect(picker.getByRole("alert")).toHaveText(
    "Your previous search is still running. Try again in a moment.",
  );
  await picker.getByRole("button", { name: "Search", exact: true }).click();
  await expect(results.getByRole("button")).toHaveCount(2);

  await results.getByRole("button").first().click();
  await expect(picker).toHaveCount(0);
  await expect(hero.getByRole("status")).toHaveText("Portrait changed.");
  expect(picks).toEqual([{ deezer_id: 76 }]);
  expect(searches).toEqual(["Björk", "Nobody", "Bjork", "Bjork"]);
  expect(thirdParty).toEqual([]);
});

test("keeps the artist hero and the Deezer picker readable in the light", async ({
  page,
}) => {
  const { state } = await serveArtistImages(page);
  await page.route("**/api/v2/artists/artist-1/image/candidates?*", (route) =>
    route.fulfill({
      json: [
        { deezer_id: 76, name: "Björk", fans: 1500000, artwork_hash: "d".repeat(64) },
      ],
    }),
  );
  await chooseLightTheme(page);
  await page.goto("/artists/artist-1");
  const hero = page.locator(".artist-hero");
  await expect(hero.locator(".artist-hero-ambient")).toBeAttached();
  await expectNoWcagViolations(page);

  // Opened from the hero, the picker sits inside it in the tree, and took
  // the hero's light-on-dark ink: the fan count was white on white.
  await (await openImageMenu(page))
    .getByRole("menuitem", { name: "Search Deezer" })
    .click();
  const picker = page.getByRole("dialog", { name: "Choose a portrait from Deezer" });
  await expect(picker.getByText("1500000 fans")).toBeVisible();
  await expectNoWcagViolations(page);
  await picker.getByRole("button", { name: "Close" }).click();

  state.banner = "c".repeat(64);
  await page.reload();
  await expect(hero.locator("img.artist-banner")).toBeVisible();
  await expectNoWcagViolations(page);
});

/**
 * The artist hero over the picture least kind to its text, in both
 * materials: a white banner under the light text a banner always takes, and
 * a portrait at the far end from the ink — white behind the dark theme's
 * light text, black behind the light theme's dark ink. Small text holds AA's
 * 4.5:1 and the title, large, its 3:1.
 */
for (const { theme, banner, portrait } of [
  { theme: "default-dark", banner: true, portrait: "#ffffff" },
  { theme: "default", banner: true, portrait: "#ffffff" },
  { theme: "default-dark", banner: false, portrait: "#ffffff" },
  { theme: "default", banner: false, portrait: "#000000" },
]) {
  test(`keeps the artist hero legible over a ${banner ? "white banner" : `${portrait} portrait`} in ${theme}`, async ({
    page,
  }) => {
    const { state } = await serveArtistImages(page);
    if (banner) state.banner = "c".repeat(64);
    // A portrait chosen by hand, so the quiet "Automatic portrait" button is
    // drawn too and measured with the others.
    state.portrait = "d".repeat(64);
    const fill = banner ? "#ffffff" : portrait;
    await page.route("**/api/v2/artwork/*", (route) =>
      route.fulfill({
        contentType: "image/svg+xml",
        body: `<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="${fill}"/></svg>`,
      }),
    );
    await page.addInitScript((id) => {
      localStorage.setItem("waveflow.theme", id);
    }, theme);
    await page.goto("/artists/artist-1");
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const hero = page.locator(".artist-hero");
    await expect(
      hero.locator(banner ? "img.artist-banner" : ".artist-hero-ambient"),
    ).toBeVisible();
    await page.evaluate(() =>
      Promise.all(
        document.getAnimations().map((animation) => animation.finished.catch(() => undefined)),
      ),
    );

    // Each text named by what it says, so that a line gone missing fails
    // here instead of handing its place to the next one of the same class.
    const name = hero.getByRole("heading", { name: "Björk", exact: true });
    const kicker = hero.locator(".eyebrow");
    const count = hero.getByText("2 albums in your library", { exact: true });
    const source = hero.getByText("Portrait chosen by hand", { exact: true });
    for (const text of [name, kicker, count, source]) {
      await expect(text).toHaveCount(1);
      await expect(text).toBeVisible();
    }
    // And no other source line beside it, left unmeasured.
    await expect(hero.locator(".artist-source")).toHaveCount(1);
    expect(await lowestContrast(page, name), "the name").toBeGreaterThanOrEqual(3);
    for (const [label, text] of [
      ["the kicker", kicker],
      ["the count", count],
      ["the source", source],
    ] as const) {
      expect(await lowestContrast(page, text), label).toBeGreaterThanOrEqual(4.5);
    }
    // Every button over the picture is measured. The image actions sit
    // behind one menu button now, which is required, so the loop below
    // cannot pass over an empty list.
    const trigger = hero.getByRole("button", { name: "Edit images", exact: true });
    await expect(trigger).toHaveCount(1);
    await expect(trigger).toBeVisible();
    const overPicture = await hero.locator(".artist-image-button").all();
    expect(overPicture.length).toBeGreaterThan(0);
    for (const button of overPicture) {
      if (!(await button.isVisible())) continue;
      expect(
        await lowestContrast(page, button, true),
        `the button “${await button.innerText()}”`,
      ).toBeGreaterThanOrEqual(4.5);
    }
    // And what it opens: the plain entry and the quiet one, both required —
    // a portrait chosen by hand is why "Automatic portrait" is offered. The
    // menu is drawn on its own panel, outside the header, and is measured
    // over whatever it covers all the same.
    await trigger.click();
    const menu = page.getByRole("menu", { name: "Images of Björk" });
    for (const required of ["Change portrait", "Automatic portrait"]) {
      const entry = menu.getByRole("menuitem", { name: required, exact: true });
      await expect(entry).toHaveCount(1);
      await expect(entry).toBeVisible();
    }
    for (const entry of await menu.getByRole("menuitem").all()) {
      expect(
        await lowestContrast(page, entry.locator("span")),
        `the menu entry “${await entry.innerText()}”`,
      ).toBeGreaterThanOrEqual(4.5);
    }
  });
}

/**
 * RFC-022 decisions 2 and 3 on the screen: the albums in the sections the
 * server filed them under, in the page's order whatever order they came in,
 * empty sections left out, and the appearances last.
 */
test("an artist page files its records by type, and lists what it appears on", async ({
  page,
}) => {
  mock.artistExtra = {
    albums: [
      { ...albums[1], id: "album-3", title: "Vespertine Live", release_group: "live" },
      { ...albums[0], release_group: "album" },
      { ...albums[1], release_group: "album" },
    ],
    appears_on: [
      {
        ...albums[0],
        id: "album-9",
        title: "Guest Spot",
        artist: "Someone Else",
        artist_id: "artist-9",
        release_group: "album",
      },
    ],
  };
  await page.goto("/artists/artist-1");
  await expect(
    page.getByRole("heading", { name: "Björk", level: 2 }),
  ).toBeVisible();
  await expect(page.locator(".artist-discography h3")).toHaveText([
    "Albums",
    "Live",
    "Appears on",
  ]);
  const appearances = page.getByRole("region", { name: "Appears on" });
  await expect(appearances.getByText("Guest Spot", { exact: true })).toBeVisible();
  await expect(appearances.getByText("Someone Else")).toBeVisible();
  // Filed under its own section, and nowhere else.
  await expect(
    page.getByRole("region", { name: "Live" }).getByText("Vespertine Live"),
  ).toBeVisible();
  await expect(
    page.getByRole("region", { name: "Albums" }).getByText("Vespertine Live"),
  ).toHaveCount(0);
  await expectNoWcagViolations(page);
});

/**
 * An older server sends no groups and no appearances: one section, the one
 * the page always had.
 */
test("an artist page from a server without release groups keeps one section", async ({
  page,
}) => {
  await page.goto("/artists/artist-1");
  await expect(page.locator(".artist-discography h3")).toHaveText(["Albums"]);
  await expect(
    page.getByRole("region", { name: "Albums" }).getByText("Vespertine", { exact: true }),
  ).toBeVisible();
});
