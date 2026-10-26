// Browsing the catalogue: songs, albums, genres, history and covers.

import { expect, test } from "@playwright/test";

import { expectNoWcagViolations } from "./fixtures/accessibility";
import { resetAndMockApi } from "./fixtures/api";
import { library, track } from "./fixtures/catalogue";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

test("puts the songs playback action beside the sort and names its scope", async ({
  page,
}) => {
  await page.route("**/api/v2/songs?*", (route) =>
    route.fulfill({
      json: [track, { ...track, id: "song-2", title: "Second track" }],
    }),
  );
  await page.goto("/songs");
  const play = page.locator(".page-header").getByRole("button", {
    name: "Play all 2 tracks",
  });
  await expect(play).toBeVisible();
  await play.click();
  await expect(page.locator(".player-track strong")).toHaveText("Army of Me");
});

/**
 * Every credited artist in a song list is a link to its own page.
 *
 * The column used to be the display string as plain text, so a playlist or a
 * mix offered no way to reach an artist. A track credited to two artists names
 * two pages; a track whose credits the server did not list still links through
 * its primary credit; an artist the catalogue does not know stays text.
 */
test("links each credited artist in a song list to its page", async ({
  page,
}, testInfo) => {
  await page.route("**/api/v2/songs?*", (route) =>
    route.fulfill({
      json: [
        {
          ...track,
          id: "song-duo",
          title: "Soleil Bleu",
          artist: "Bleu Soleil, LUIZA",
          artist_id: "artist-1",
          artists: [
            { id: "artist-1", name: "Bleu Soleil" },
            { id: "artist-2", name: "LUIZA" },
          ],
        },
        {
          ...track,
          id: "song-primary",
          title: "Second track",
          artist: "Björk",
          artist_id: "artist-1",
          artists: [],
        },
        {
          ...track,
          id: "song-unknown",
          title: "Third track",
          artist: "Nobody We Know",
          artist_id: null,
          artists: [],
        },
      ],
    }),
  );
  await page.goto("/songs");
  const rows = page.locator(".songs tbody tr");
  await expect(rows).toHaveCount(3);

  // Every layout: the track's menu names each credited artist, which is how a
  // phone, whose song table hides the artist column, reaches one.
  await rows.nth(0).locator("button.song-title").focus();
  await page.keyboard.press("Shift+F10");
  const menu = page.getByRole("menu", { name: "Actions for Soleil Bleu" });
  await expect(menu.getByRole("menuitem", { name: /^Go to / })).toHaveText([
    "Go to album",
    "Go to Bleu Soleil",
    "Go to LUIZA",
  ]);
  await menu.getByRole("menuitem", { name: "Go to LUIZA" }).click();
  await expect(page).toHaveURL(/\/artists\/artist-2$/);
  await page.goBack();
  await expect(rows).toHaveCount(3);

  // The column itself is drawn on a wide screen only.
  if (testInfo.project.name !== "desktop") return;
  const duo = rows.nth(0).locator(".song-artists");
  await expect(duo.getByRole("link")).toHaveCount(2);
  await expect(duo).toHaveText("Bleu Soleil, LUIZA");
  await expect(duo.getByRole("link", { name: "LUIZA" })).toHaveAttribute(
    "href",
    "/artists/artist-2",
  );

  await expect(
    rows.nth(1).locator(".song-artists").getByRole("link", { name: "Björk" }),
  ).toHaveAttribute("href", "/artists/artist-1");
  await expect(rows.nth(2).locator(".song-artists").getByRole("link")).toHaveCount(
    0,
  );
  await expect(rows.nth(2).locator(".song-artists")).toHaveText("Nobody We Know");

  await expectNoWcagViolations(page);

  await duo.getByRole("link", { name: "Bleu Soleil" }).click();
  await expect(page).toHaveURL(/\/artists\/artist-1$/);
});

/**
 * A track menu taller than the window stays on screen and scrolls itself.
 *
 * One "Go to" entry per credited artist means a compilation track can carry a
 * menu longer than a short window. It is capped at the window and scrolls on
 * its own; the wheel over it scrolls it rather than closing it, and the wheel
 * over the page still closes it.
 */
test("keeps a long track menu on screen and lets it scroll", async ({
  page,
}) => {
  await page.setViewportSize({ width: 900, height: 420 });
  const many = Array.from({ length: 30 }, (_, index) => ({
    id: `artist-${index + 1}`,
    name: `Guest ${index + 1}`,
  }));
  await page.route("**/api/v2/songs?*", (route) =>
    route.fulfill({
      json: [
        {
          ...track,
          id: "song-crowd",
          title: "Crowded",
          artist: many.map((artist) => artist.name).join(", "),
          artist_id: "artist-1",
          artists: many,
        },
      ],
    }),
  );
  await page.goto("/songs");
  await page.locator(".songs tbody tr button.song-title").first().focus();
  await page.keyboard.press("Shift+F10");
  const menu = page.getByRole("menu", { name: "Actions for Crowded" });
  await expect(menu).toBeVisible();

  const box = await menu.boundingBox();
  expect(box).not.toBeNull();
  expect((box?.y ?? 0) + (box?.height ?? 0)).toBeLessThanOrEqual(420);
  expect(await menu.evaluate((node) => node.scrollHeight > node.clientHeight)).toBe(
    true,
  );

  // The wheel over the menu scrolls it and leaves it open.
  await menu.hover();
  await page.mouse.wheel(0, 400);
  await expect
    .poll(() => menu.evaluate((node) => node.scrollTop))
    .toBeGreaterThan(0);
  await expect(menu).toBeVisible();
  const last = menu.getByRole("menuitem", { name: "Go to Guest 30" });
  await last.scrollIntoViewIfNeeded();
  await expect(last).toBeInViewport();

  // The wheel anywhere else still closes it.
  await page.mouse.move(880, 400);
  await page.mouse.wheel(0, 100);
  await expect(menu).toHaveCount(0);
});

test("gives random playback one clear primary action", async ({ page }) => {
  await page.route("**/api/v2/songs/random?*", (route) =>
    route.fulfill({ json: [track] }),
  );
  await page.goto("/random");
  const controls = page.locator(".page-header");
  const play = controls.getByRole("button", { name: "Play", exact: true });
  const queue = controls.getByRole("button", { name: "Add queue" });
  const redraw = controls.getByRole("button", { name: "Draw again" });
  await expect(play).toBeVisible();
  await expect(queue).toBeVisible();
  await expect(redraw).toBeVisible();
  const playFill = await play.evaluate((node) => getComputedStyle(node).backgroundColor);
  const queueFill = await queue.evaluate((node) => getComputedStyle(node).backgroundColor);
  expect(playFill).not.toBe(queueFill);
});

/**
 * The browse controls are the whole of lot A that has behaviour rather than
 * appearance. Sorting has to reach the server — it is `AlbumOrder` there, and
 * four of its values filter as well as order — while filtering must not, since
 * the client already holds the list.
 */
test("sorts through the server and filters in the browser", async ({
  page,
}) => {
  const requested: Array<string | null> = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname === "/api/v2/albums") {
      requested.push(url.searchParams.get("sort"));
    }
  });

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();

  const titles = page.locator(".grid strong");
  await expect(titles).toHaveText(["Post", "Vespertine"]);
  // The page names its order on the first load rather than relying on the
  // server default, so the menu and the request cannot disagree.
  expect(requested).toEqual(["alphabeticalByName"]);

  await page.getByLabel("Sort").selectOption("newest");
  await expect(titles).toHaveText(["Vespertine", "Post"]);
  expect(requested).toEqual(["alphabeticalByName", "newest"]);

  // Filtering narrows what is already loaded: no further request is made, and
  // the header count reports the subset against the whole.
  await page.getByLabel("Filter by title or artist").fill("vesper");
  await expect(titles).toHaveText(["Vespertine"]);
  await expect(page.getByText("1 of 2 shown")).toBeVisible();
  expect(requested).toEqual(["alphabeticalByName", "newest"]);

  // A filter matching nothing says so instead of showing an empty grid.
  await page.getByLabel("Filter by title or artist").fill("zzz");
  await expect(page.getByText("Nothing matches that filter.")).toBeVisible();
});

/**
 * Covers are asked for when they are about to be seen, not when the page mounts.
 *
 * `/api/v2/artwork/{hash}` needs an `Authorization` header, so each thumbnail
 * costs a `fetch` of its own. A library of 164 tracks opened with 127 requests
 * in half a second against a connection limit of six, and the covers below the
 * fold were queued ahead of the ones being looked at.
 *
 * Asserted on one card at the far end rather than on a count: a count depends
 * on the viewport and would have to be loosened until it stopped meaning
 * anything. What this pins is the property — off screen is not asked for, and
 * scrolling to it asks.
 */
test("asks for a cover when it comes into view, and not before", async ({
  page,
}) => {
  const asked: string[] = [];
  const shelf = Array.from({ length: 60 }, (_, index) => ({
    id: `album-lazy-${index}`,
    library_id: "library-1",
    title: `Shelf ${String(index).padStart(2, "0")}`,
    artist: "Rue Delacour",
    artist_id: "artist-1",
    artwork_hash: `hash-${index}`,
    year: 2000 + index,
    starred_at: null,
    user_rating: null,
  }));

  // Registered after `mockAuthenticatedApi`, so these win: Playwright tries
  // handlers newest first.
  await page.route("**/api/v2/albums*", async (route) => {
    await route.fulfill({ json: shelf });
  });
  await page.route("**/api/v2/artwork/*", async (route) => {
    asked.push(new URL(route.request().url()).pathname.split("/").pop() ?? "");
    // A 404 is enough: this is about which requests leave, not what comes back.
    await route.fulfill({ status: 404, body: "" });
  });

  await page.goto("/");
  const last = page.getByText("Shelf 59");
  await expect(last).toBeVisible();

  // The top of the shelf is on screen and has been asked for; the bottom is
  // sixty cards away and has not.
  await expect.poll(() => asked).toContain("hash-0");
  expect(asked).not.toContain("hash-59");

  await last.scrollIntoViewIfNeeded();
  await expect.poll(() => asked).toContain("hash-59");
});

/**
 * The card actions guard themselves while their album's tracks are being
 * fetched. The guard was one album id, which meant two cards in flight shared
 * a single slot: pressing the second card cleared the first one's guard
 * outright, and its buttons came back to life with its own request still out.
 *
 * That mattered because one of the two actions appends. A second press on "add
 * to queue" while the first was unanswered queued the album twice.
 */
test("keeps each card's actions guarded while its own fetch is out", async ({
  page,
}) => {
  let releaseSlow = () => {};
  mock.slowAlbum = new Promise<void>((resolve) => {
    releaseSlow = resolve;
  });

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();

  const slowPlay = page.getByRole("button", { name: "Play: Post" });
  const fastQueue = page.getByRole("button", {
    name: "Add to queue: Vespertine",
  });

  await page.locator(".grid li").first().hover();
  await slowPlay.click();
  await expect(slowPlay).toBeDisabled();

  // The second album answers at once while the first is still held open.
  // Synchronising on the response and not on the button is the point: the
  // button is enabled before the click too, so `toBeEnabled` can resolve on
  // its first poll, before React has even applied the disabling update — and
  // the assertion below would then run at a moment when nothing has happened,
  // which is exactly when the single-slot guard still looks correct.
  const answered = page.waitForResponse(
    (response) => new URL(response.url()).pathname === "/api/v2/albums/album-2",
  );
  await page.locator(".grid li").nth(1).hover();
  await fastQueue.click();
  await answered;
  await expect(fastQueue).toBeEnabled();

  // The first album has not answered, so its actions must still be refused.
  await expect(slowPlay).toBeDisabled();

  releaseSlow();
  await expect(slowPlay).toBeEnabled();
});

test("album view plays its tracks and keeps its compact list readable", async ({
  page,
}) => {
  // "More by" reads the artist's albums alone. The library listing would page
  // through every album to keep six, and the artist's page asks a metadata
  // provider; neither has any business being asked from here.
  const listings: string[] = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (path === "/api/v2/albums" || path === "/api/v2/artists/artist-1") {
      listings.push(path);
    }
  });
  await page.goto("/albums/album-2");
  await expect(page.locator(".album-hero h2")).toHaveText("Vespertine");
  await expect(page.locator(".album-meta")).toContainText("3 tracks");
  await expect(page.locator(".album-meta")).toContainText("15:00");
  await expect(page.locator(".album-songs tbody tr")).toHaveCount(3);
  await expect(page.locator(".album-songs .song-cover")).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "More by Björk" })).toBeVisible();
  await expect(page.locator(".album-related-grid a")).toHaveCount(1);
  expect(listings).toEqual([]);
  const actions = await page.locator(".album-actions button").evaluateAll((buttons) =>
    buttons.map((button) => {
      const style = getComputedStyle(button);
      return {
        left: Number.parseFloat(style.paddingLeft),
        right: Number.parseFloat(style.paddingRight),
      };
    }),
  );
  expect(actions).toHaveLength(2);
  for (const action of actions) {
    expect(action.left).toBeGreaterThanOrEqual(20);
    expect(action.right).toBeGreaterThanOrEqual(20);
  }

  await page.getByRole("button", { name: "Play album" }).click();
  await expect(page.locator(".player-track strong")).toHaveText("Hidden Place");
  await page.locator(".album-songs .song-title").nth(1).click();
  await expect(page.locator(".player-track strong")).toHaveText("Cocoon");
  expect(
    await page.evaluate(() =>
      document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
});

/**
 * The song table carries the controls the albums grid does not — a five-star
 * rating and a favourite — so the accessibility sweep has to reach a page that
 * shows one. Until this test the sweep only ever loaded the grid.
 */
test("rates a track and stays free of WCAG A or AA violations", async ({
  page,
}) => {
  const rated: string[] = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname.startsWith("/api/v2/ratings/")) {
      rated.push(`${request.method()} ${url.pathname}`);
    }
  });

  await page.goto("/albums/album-2");
  await expect(page.getByRole("heading", { name: "Vespertine" })).toBeVisible();

  // The rating is a radio group, so the stored value is a checked radio rather
  // than a class on a span.
  const hidden = page.getByRole("group", { name: "Rating: Hidden Place" });
  await expect(hidden.getByRole("radio", { name: "5 stars" })).toBeChecked();
  const cocoon = page.getByRole("group", { name: "Rating: Cocoon" });
  await expect(cocoon.getByRole("radio", { checked: true })).toHaveCount(0);

  await cocoon.getByRole("radio", { name: "4 stars" }).check();
  await expect(cocoon.getByRole("radio", { name: "4 stars" })).toBeChecked();
  expect(rated).toEqual(["PUT /api/v2/ratings/track/song-2"]);

  await expectNoWcagViolations(page);
});

/**
 * Genres were a route the server answered and the client never called. The
 * navigation is the part worth pinning: the genre name travels into the query,
 * and it is a display string — "Hip-Hop" and "hip hop" are one genre to the
 * server, which canonicalises before matching.
 */
test("browses into a genre and keeps WCAG A and AA clean", async ({ page }) => {
  const asked: Array<string | null> = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname === "/api/v2/songs/by-genre") {
      asked.push(url.searchParams.get("genre"));
    }
  });

  await page.goto("/genres");
  await expect(page.getByRole("heading", { name: "Genres" })).toBeVisible();
  await expect(page.getByText("412 tracks · 31 albums")).toBeVisible();

  await expectNoWcagViolations(page);

  await page.getByRole("link", { name: "Art Pop" }).click();
  await expect(page.getByRole("heading", { name: "Art Pop" })).toBeVisible();
  await expect(page.locator(".songs tbody tr")).toHaveCount(2);
  expect(asked).toEqual(["Art Pop"]);

  await expectNoWcagViolations(page);
});

/**
 * `GET /history` answers plays and not songs, so every distinct track on this
 * screen is a request of its own, and they all leave together. Without a cap a
 * long history meant a hundred-odd round trips for a list nobody reads to the
 * bottom of. The window stays wide — 200 plays asked for — and only what is
 * resolved is bounded.
 */
test("resolves a bounded number of tracks from a long history", async ({
  page,
}) => {
  // The track itself and nothing under it: `/tracks/{id}/stream-ticket` shares
  // the prefix, and the restored queue asks for those as soon as it loads.
  const asked: string[] = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (/^\/api\/v2\/tracks\/[^/]+$/.test(path)) asked.push(path);
  });

  await page.goto("/history");
  await expect(
    page.getByRole("heading", { name: "Recently played" }),
  ).toBeVisible();

  await expect(page.locator(".songs tbody tr")).toHaveCount(50);
  expect(asked).toHaveLength(50);
  // The newest plays are the ones kept, not an arbitrary fifty.
  expect(asked[0]).toBe("/api/v2/tracks/t0");
  expect(asked.at(-1)).toBe("/api/v2/tracks/t49");
});

/**
 * The web client works inside one library at a time — decided 2026-09-07, and
 * the contract is that changing library changes the catalogue's scope rather
 * than merging catalogues. What is worth pinning is that the scope actually
 * travels: a picker that changed nothing on the wire would look right and be
 * wrong.
 */
test("scopes the catalogue to the active library, and hides the picker when there is one", async ({
  page,
}, testInfo) => {
  const scopes: Array<string | null> = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname === "/api/v2/albums") {
      scopes.push(url.searchParams.get("library_id"));
    }
  });

  mock.libraries = [
    library("library-1", "Ma musique"),
    library("library-2", "Les enfants"),
  ];

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();

  // The picker exists in the sidebar and in the mobile header; only one of the
  // two is on screen, so the locator has to say which — as the appearance test
  // in `shell.spec.ts` ("renders the studio shell and persists appearance
  // preferences") already does for theme and language.
  const chrome =
    testInfo.project.name === "mobile"
      ? page.locator(".mobile-header")
      : page.locator(".sidebar");
  const picker = chrome.getByLabel("Library");
  await expect(picker).toBeVisible();
  // The first library leads until one has been chosen.
  await expect.poll(() => scopes.at(-1)).toBe("library-1");

  await picker.selectOption("library-2");
  await expect.poll(() => scopes.at(-1)).toBe("library-2");

  // Remembered across a reload rather than reset to the first.
  await page.reload();
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect.poll(() => scopes.at(-1)).toBe("library-2");
});

test("shows no library picker when the account has only one", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  // A select with a single option is a control that does nothing.
  await expect(page.getByLabel("Library")).toHaveCount(0);
});

test("keeps the catalogue unasked until the active library is known", async ({
  page,
}) => {
  // The first render used to fire an unscoped request and then a scoped one,
  // so for a moment the screen showed every library's albums — the exact thing
  // the single-library scope exists to prevent.
  const scopes: Array<string | null> = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname === "/api/v2/albums") {
      scopes.push(url.searchParams.get("library_id"));
    }
  });

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(page.getByText("Post", { exact: true })).toBeVisible();
  expect(scopes).toEqual(["library-1"]);
});
