// What the library keeps on screen, and when it asks the server again.

import { expect, test } from "@playwright/test";

import { resetAndMockApi } from "./fixtures/api";
import { albums, session, track } from "./fixtures/catalogue";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

/** A tab switch keeps the current screen until the destination can render. */
test("keeps the current tab visible while the next tab loads", async ({
  page,
}) => {
  const asked: string[] = [];
  let answerGenres: (() => void) | undefined;
  const genresReady = new Promise<void>((resolve) => {
    answerGenres = resolve;
  });
  await page.route("**/api/v2/genres*", async (route) => {
    asked.push(new URL(route.request().url()).pathname);
    await genresReady;
    await route.fulfill({
      json: [{ name: "Trip hop", song_count: 4, album_count: 1 }],
    });
  });
  await page.route("**/api/v2/albums*", async (route) => {
    asked.push(new URL(route.request().url()).pathname);
    await route.fulfill({ json: albums });
  });

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  const mobile = await page.locator(".mobile-navigation").isVisible();
  const genresLink = mobile
    ? page.locator('.more-sheet a[href="/genres"]')
    : page.locator(".primary-navigation").getByRole("link", { name: "Genres" });
  async function visitGenres() {
    if (mobile) {
      await page.locator(".mobile-navigation").getByRole("button", { name: "More" }).click();
    }
    await genresLink.click();
  }
  await visitGenres();
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(page.locator(".skeleton-grid")).toHaveCount(0);
  await expect(genresLink).toHaveAttribute(
    "aria-busy",
    "true",
  );
  answerGenres?.();
  await expect(page.getByRole("heading", { name: "Genres" })).toBeVisible();
  await page.getByRole("link", { name: "Albums" }).click();
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await visitGenres();
  await expect(page.getByRole("heading", { name: "Genres" })).toBeVisible();
  await expect(page.locator(".skeleton-grid")).toHaveCount(0);
  expect(asked).toEqual(["/api/v2/albums", "/api/v2/genres"]);
});

/**
 * A tab switch waits for its answer, but not for ever.
 *
 * Past a short deadline the destination is shown with its own wait. A server
 * that never answered held the previous screen, with nothing but a busy tab
 * to say anything was happening.
 */
test("shows the next tab with its wait when the answer is slow", async ({
  page,
}) => {
  // Never answered: the deadline is the only way out.
  await page.route("**/api/v2/genres*", () => new Promise<void>(() => {}));
  await page.route("**/api/v2/albums*", (route) =>
    route.fulfill({ json: albums }),
  );

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  const mobile = await page.locator(".mobile-navigation").isVisible();
  if (mobile) {
    await page
      .locator(".mobile-navigation")
      .getByRole("button", { name: "More" })
      .click();
  }
  const genresLink = mobile
    ? page.locator('.more-sheet a[href="/genres"]')
    : page.locator(".primary-navigation").getByRole("link", { name: "Genres" });
  await genresLink.click();
  await expect(page).toHaveURL(/\/genres$/);
  // The destination's own wait, which is what the visitor now looks at.
  await expect(page.locator("main .skeleton-grid")).toBeVisible();
});

/**
 * The current tab's link still goes to its plain address.
 *
 * A click on the tab already open is how a second click cancels a switch
 * still loading. With nothing loading, swallowing it left `/playing?view=lyrics`
 * where it was, though the link says `/playing`.
 */
test("takes the current tab back to its plain address", async ({ page }) => {
  await page.goto("/playing?view=lyrics");
  await expect(page).toHaveURL(/\/playing\?view=lyrics$/);
  const navigation = (await page.locator(".mobile-navigation").isVisible())
    ? page.locator(".mobile-navigation")
    : page.locator(".primary-navigation");
  await navigation.getByRole("link", { name: "Now playing" }).click();
  await expect(page).toHaveURL(/\/playing$/);
});

/**
 * The random tab draws once.
 *
 * Its answer is held for no time at all, so a draw made ahead of the switch
 * was thrown away before the page mounted, and the page drew again: two
 * requests, and a wait for both.
 */
test("draws the random tab once when switching to it", async ({ page }) => {
  let draws = 0;
  await page.route("**/api/v2/songs/random*", async (route) => {
    draws += 1;
    await route.fulfill({ json: [track] });
  });
  await page.route("**/api/v2/albums*", (route) =>
    route.fulfill({ json: albums }),
  );

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  if (await page.locator(".mobile-navigation").isVisible()) {
    await page.locator(".mobile-navigation").getByRole("button", { name: "More" }).click();
    await page.locator('.more-sheet a[href="/random"]').click();
  } else {
    await page.locator(".primary-navigation").getByRole("link", { name: "Random" }).click();
  }
  await expect(page.getByRole("heading", { name: "Random" })).toBeVisible();
  await expect(page.locator(".songs")).toContainText("Army of Me");
  // Long enough for a second draw from the mount to have been asked.
  await page.waitForTimeout(500);
  expect(draws).toBe(1);
});

/**
 * A tab that is still loading does not overrule a later move.
 *
 * The switch waits for the destination's first answer, and the page stays
 * usable meanwhile. Following a link in it during that wait is a newer
 * decision than the tab: arriving on the tab afterwards took the visitor away
 * from the album they had just opened.
 */
test("leaves a tab switch behind when the visitor moves on while it loads", async ({
  page,
}) => {
  let answerGenres: (() => void) | undefined;
  const genresReady = new Promise<void>((resolve) => {
    answerGenres = resolve;
  });
  let genresAnswered = false;
  await page.route("**/api/v2/genres*", async (route) => {
    await genresReady;
    await route.fulfill({
      json: [{ name: "Trip hop", song_count: 4, album_count: 1 }],
    });
    genresAnswered = true;
  });
  await page.route("**/api/v2/albums*", (route) =>
    route.fulfill({ json: albums }),
  );

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  const mobile = await page.locator(".mobile-navigation").isVisible();
  if (mobile) {
    await page.locator(".mobile-navigation").getByRole("button", { name: "More" }).click();
  }
  const genresLink = mobile
    ? page.locator('.more-sheet a[href="/genres"]')
    : page.locator(".primary-navigation").getByRole("link", { name: "Genres" });
  await genresLink.click();
  await expect(genresLink).toHaveAttribute("aria-busy", "true");

  await page.locator('.grid a[href="/albums/album-1"]').click();
  await expect(page).toHaveURL(/\/albums\/album-1$/);

  answerGenres?.();
  await expect.poll(() => genresAnswered).toBe(true);
  await expect(genresLink).not.toHaveAttribute("aria-busy", "true");
  // Long enough for the navigation the answer used to trigger to land.
  await page.waitForTimeout(500);
  await expect(page).toHaveURL(/\/albums\/album-1$/);
  await expect(page.getByRole("heading", { name: "Genres" })).toHaveCount(0);
});

/**
 * Signing out empties what was answered for the account that left.
 *
 * The cache is the first thing in this client that outlives a screen, and a
 * sign-out here is a client-side navigation — the document is never reloaded,
 * so nothing is forgotten on its own. A catalogue is scoped by library
 * membership and an administrator sees more than that, so carrying any of it
 * across would show the next person what the last one could see.
 *
 * Asserted on the request, because that is the only thing that distinguishes an
 * emptied cache from a full one: both render the same albums, and only one of
 * them had to ask.
 */
test("forgets the catalogue when the session changes", async ({
  page,
}, testInfo) => {
  // The sidebar that carries the sign-out is hidden below 820px, where the
  // same gesture lives behind the overflow sheet. What is being measured is
  // the cache, not where the button sits, so it is measured once.
  test.skip(testInfo.project.name !== "desktop", "about the sidebar");
  let asked = 0;
  await page.route("**/api/v2/albums*", async (route) => {
    asked += 1;
    await route.fulfill({ json: albums });
  });
  await page.route("**/api/v2/web/auth/login", async (route) => {
    await route.fulfill({ json: session });
  });

  await page.goto("/");
  await expect(page.getByText("Post", { exact: true })).toBeVisible();
  expect(asked).toBe(1);

  // Away and straight back, without signing out: this is the cache doing its
  // job, and the contrast that makes the count below mean something.
  const sidebar = page.getByRole("navigation");
  await sidebar.getByRole("link", { name: "Queue" }).click();
  await expect(page.getByRole("heading", { name: "Queue" })).toBeVisible();
  await sidebar.getByRole("link", { name: "Albums" }).click();
  await expect(page.getByText("Post", { exact: true })).toBeVisible();
  expect(asked).toBe(1);

  // Dispatched rather than clicked: the sidebar's footer sits under the fixed
  // player bar at this viewport, so a real click waits on hit-testing forever.
  // What this test is about is what the sign-out does to the cache, and the
  // gesture itself is covered by the accessibility pass.
  await page
    .getByRole("button", { name: "Sign out" })
    .dispatchEvent("click");
  await expect(page).toHaveURL(/\/login$/);

  // Signed in again through the form, not by reloading the page. A reload
  // would destroy the cache whatever this code does, and would prove nothing:
  // the leak this guards against is exactly the one that needs no reload.
  await page.getByLabel("Username").fill("nocturne");
  await page.getByLabel("Password").fill("correct horse battery staple");
  await page.getByRole("button", { name: "Sign in" }).click();

  await expect(page.getByText("Post", { exact: true })).toBeVisible();
  expect(asked).toBe(2);
});

/**
 * A cover already in memory does not flash grey on the way back.
 *
 * The bytes are held in a map keyed by hash, but that map holds *promises*, so
 * nothing in it could be read while rendering: every mount started with no
 * image and put the placeholder on screen for a frame — on every navigation
 * back to a grid already visited, for every cover in it. Navidrome's own image
 * cache reads its entry synchronously into `useState` for exactly this reason.
 *
 * Watched frame by frame rather than checked once, because a single frame is
 * the whole of what went wrong: an assertion that ran after the navigation
 * settled would have found the covers in place and called it fixed.
 */
test("shows a held cover at once, without a grey frame in between", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "about the sidebar");

  const shelf = albums.map((album, index) => ({
    ...album,
    artwork_hash: `hash-${index}`,
  }));
  await page.route("**/api/v2/albums*", async (route) => {
    await route.fulfill({ json: shelf });
  });
  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
    "base64",
  );
  await page.route("**/api/v2/artwork/*", async (route) => {
    await route.fulfill({ contentType: "image/png", body: png });
  });

  await page.goto("/");
  // Both covers in place, so their bytes are held before the round trip below.
  await expect(page.locator("img.cover")).toHaveCount(2);

  const sidebar = page.getByRole("navigation");
  await sidebar.getByRole("link", { name: "Queue" }).click();
  await expect(page.getByRole("heading", { name: "Queue" })).toBeVisible();

  const greyFrames = await page.evaluate(async () => {
    const link = [...document.querySelectorAll("nav a")].find((anchor) =>
      anchor.textContent?.includes("Albums"),
    ) as HTMLElement;
    let grey = 0;
    link.click();
    const started = performance.now();
    return await new Promise<number>((resolve) => {
      const tick = () => {
        grey += document.querySelectorAll(".grid .cover-fallback").length;
        if (performance.now() - started > 400) return resolve(grey);
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    });
  });

  // The catalogue is back and showing its covers. Asserted after the count and
  // not instead of it: zero grey frames is also what a navigation that never
  // arrived would produce, and this assertion is the whole difference between
  // measuring the fix and measuring nothing at all.
  await expect(page.locator(".grid img.cover")).toHaveCount(2);
  expect(greyFrames).toBe(0);
});

/**
 * A favourite asks the catalogue again, so the star survives the next visit.
 *
 * The table keeps its stars in a map of its own while the request is out, which
 * was enough when every screen re-read on mount: the server's answer replaced
 * that overlay on the way back. Against a cache it is not — the overlay dies
 * with the component and the held listing still carries the old `starred_at`,
 * so the star un-ticks itself on the next visit.
 *
 * Asserted on the request rather than on the star, because the mock answers a
 * fixture that never changes: what marks the difference is that the question
 * was asked at all.
 */
test("asks the catalogue again after a track is favourited", async ({
  page,
}) => {
  let asked = 0;
  await page.route("**/api/v2/albums/album-1", async (route) => {
    asked += 1;
    await route.fulfill({ json: { ...albums[0], songs: [track] } });
  });
  // The star only invalidates once the server has taken it. Answered here
  // because the harness has no favourites route, and a refused write must not
  // invalidate anything — which is the other half of what this checks.
  await page.route("**/api/v2/favorites/track/*", async (route) => {
    await route.fulfill({ status: 204, body: "" });
  });

  await page.goto("/albums/album-1");
  await expect(page.getByRole("heading", { name: "Post" })).toBeVisible();
  expect(asked).toBe(1);

  await page
    .getByRole("button", { name: "Add favourite: Army of Me" })
    .click();
  await expect
    .poll(() => asked, { message: "the album was re-asked" })
    .toBe(2);
});

/**
 * A cover that failed to load is asked for again; one the server does not hold
 * is not.
 *
 * The two used to be the same answer: any response that was not `ok`, and any
 * network failure, became `null` and was then held for the session — so a
 * single blip left a cover grey until the tab was reloaded. A 404 is the server
 * saying it holds no such artwork, which is worth keeping; anything else is a
 * server that could not answer, which is not.
 */
test("asks again for a cover that failed, and not for one that is absent", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "about the sidebar");

  const shelf = albums.map((album, index) => ({
    ...album,
    artwork_hash: index === 0 ? "hash-flaky" : "hash-absent",
  }));
  await page.route("**/api/v2/albums*", async (route) => {
    await route.fulfill({ json: shelf });
  });

  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
    "base64",
  );
  let flaky = 0;
  let absent = 0;
  await page.route("**/api/v2/artwork/hash-flaky", async (route) => {
    flaky += 1;
    // Down the first time, up afterwards.
    if (flaky === 1) return route.fulfill({ status: 503, body: "" });
    await route.fulfill({ contentType: "image/png", body: png });
  });
  await page.route("**/api/v2/artwork/hash-absent", async (route) => {
    absent += 1;
    await route.fulfill({ status: 404, body: "" });
  });

  await page.goto("/");
  // Waited on rather than sampled: the covers are asked for once the observer
  // says they are on screen, which is a frame or two after the grid renders.
  await expect.poll(() => flaky).toBe(1);
  await expect.poll(() => absent).toBe(1);
  await expect(page.locator(".grid .cover-fallback")).toHaveCount(2);

  const sidebar = page.getByRole("navigation");
  await sidebar.getByRole("link", { name: "Queue" }).click();
  await expect(page.getByRole("heading", { name: "Queue" })).toBeVisible();
  await sidebar.getByRole("link", { name: "Albums" }).click();

  // The one that failed is asked again and arrives; the one the server does not
  // hold is not asked again, because that answer has not changed.
  await expect(page.locator(".grid img.cover")).toHaveCount(1);
  await expect.poll(() => flaky).toBe(2);
  expect(absent).toBe(1);
});

/**
 * A correction reaches the listings that were holding the old value.
 *
 * Correcting a track can change its title, its artists, its genres, its year
 * and its numbering — so it can change what an album holds, how a listing
 * orders, which genre a track belongs to and what a search matches. While every
 * screen re-read on mount the question did not arise; against a cache, a
 * listing sits on the old answer for as long as it is held.
 *
 * **Reached by clicking throughout.** A `goto` to the editor would load a fresh
 * document, and a fresh document has an empty cache — the listing would be
 * re-asked whether or not anything was invalidated, and the test would be
 * measuring the reload. That is what defeated the first attempt at this.
 *
 * Asserted on the request, because the mock answers a fixture that does not
 * change: what marks the difference is that the question was asked again.
 */
test("re-asks a held listing after a correction is saved", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "about the sidebar");

  let asked = 0;
  await page.route("**/api/v2/albums*", async (route) => {
    asked += 1;
    await route.fulfill({ json: albums });
  });
  // The album holds the correctable track, so its editor is a link away rather
  // than an address away.
  await page.route("**/api/v2/albums/album-1", async (route) => {
    await route.fulfill({
      json: { ...albums[0], songs: [mock.correctable.song] },
    });
  });

  await page.goto("/");
  await expect(page.getByText("Post", { exact: true })).toBeVisible();
  expect(asked).toBe(1);

  const sidebar = page.getByRole("navigation");
  await page.getByRole("link", { name: /Post/ }).first().click();
  await page
    .getByRole("link", { name: "Correct tags: Army Of Me" })
    .click();

  const title = page.getByLabel("Title", { exact: true });
  await expect(title).toHaveValue("Army Of Me");
  await title.fill("Army of Me");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByText("Saved.")).toBeVisible();

  await sidebar.getByRole("link", { name: "Albums" }).click();
  await expect(page.getByText("Post", { exact: true })).toBeVisible();
  expect(asked).toBe(2);
});
