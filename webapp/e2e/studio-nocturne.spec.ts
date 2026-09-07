import AxeBuilder from "@axe-core/playwright";
import { expect, type Page, test } from "@playwright/test";

const session = {
  access_token: "e2e-access-token",
  user: { id: "user-1", username: "listener", role: "admin" },
  device_id: "device-1",
};

const albums = [
  {
    id: "album-1",
    library_id: "library-1",
    title: "Post",
    artist: "Björk",
    artist_id: "artist-1",
    artwork_hash: null,
    year: 1995,
    starred_at: null,
    user_rating: null,
  },
  {
    id: "album-2",
    library_id: "library-1",
    title: "Vespertine",
    artist: "Björk",
    artist_id: "artist-1",
    artwork_hash: null,
    year: 2001,
    starred_at: null,
    user_rating: null,
  },
];

const track = {
  id: "song-1",
  library_id: "library-1",
  album_id: "album-1",
  title: "Army of Me",
  album: "Post",
  artist: "Björk",
  artist_id: "artist-1",
  artwork_hash: null,
  duration_ms: 234_000,
  track: 1,
  disc: 1,
  starred_at: null,
  user_rating: null,
};

/**
 * 120 distinct plays, newest first. More than the screen resolves, which is the
 * point: each distinct track costs its own request.
 */
const history = Array.from({ length: 120 }, (_, index) => ({
  track_id: `t${index}`,
  submission: true,
  played_at: 1_000_000 - index,
}));

/** Resolved by default; one test replaces it to stall `album-1`. */
let slowAlbum: Promise<void> = Promise.resolve();

const song = (
  index: number,
  title: string,
  rating: number,
  starred: boolean,
) => ({
  id: `song-${index}`,
  library_id: "library-1",
  album_id: "album-2",
  title,
  album: "Vespertine",
  artist: "Björk",
  artist_id: "artist-1",
  artwork_hash: null,
  duration_ms: 300_000,
  track: index,
  disc: 1,
  starred_at: starred ? 1 : null,
  user_rating: rating,
});

const albumDetail = {
  ...albums[1],
  songs: [
    song(1, "Hidden Place", 5, true),
    song(2, "Cocoon", 0, false),
    song(3, "Undo", 2, false),
  ],
};

const genres = [
  { name: "Art Pop", song_count: 412, album_count: 31 },
  { name: "Shoegaze", song_count: 233, album_count: 18 },
];

const genreSongs = [
  song(1, "Hidden Place", 5, true),
  song(2, "Cocoon", 0, false),
];

async function mockAuthenticatedApi(page: Page) {
  await page.context().addCookies([
    {
      name: "waveflow-csrf",
      value: "e2e-csrf",
      url: "http://127.0.0.1:4173",
    },
  ]);
  await page.route("**/api/v2/**", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/api/v2/web/auth/refresh") {
      await route.fulfill({ json: session });
      return;
    }
    if (url.pathname === "/api/v2/albums/album-1") {
      // Held open by the concurrency test; instant for everyone else.
      await slowAlbum;
      await route.fulfill({ json: { ...albums[0], songs: [track] } });
      return;
    }
    if (url.pathname === "/api/v2/queue") {
      await route.fulfill({
        json: {
          current: "song-1",
          position_ms: 0,
          changed_by: null,
          updated_at: 1,
          songs: albumDetail.songs,
        },
      });
      return;
    }
    if (url.pathname === "/api/v2/history") {
      await route.fulfill({ json: history });
      return;
    }
    if (url.pathname.startsWith("/api/v2/tracks/")) {
      const id = url.pathname.split("/")[4];
      await route.fulfill({ json: { ...song(1, `Track ${id}`, 0, false), id } });
      return;
    }
    if (url.pathname === "/api/v2/genres") {
      await route.fulfill({ json: genres });
      return;
    }
    if (url.pathname === "/api/v2/songs") {
      await route.fulfill({
        json: url.searchParams.get("genre") ? genreSongs : [],
      });
      return;
    }
    if (url.pathname === "/api/v2/albums/album-2") {
      await route.fulfill({ json: albumDetail });
      return;
    }
    if (url.pathname.startsWith("/api/v2/ratings/")) {
      await route.fulfill({ status: 204, body: "" });
      return;
    }
    if (url.pathname === "/api/v2/albums") {
      // Answering the order lets a test tell a refetch from a local re-sort.
      const sorted =
        url.searchParams.get("sort") === "newest"
          ? [...albums].reverse()
          : albums;
      await route.fulfill({ json: sorted });
      return;
    }
    await route.fulfill({ status: 404, json: { error: "not found" } });
  });
}

test.beforeEach(async ({ page }) => {
  slowAlbum = Promise.resolve();
  await mockAuthenticatedApi(page);
});

test("renders the studio shell and persists appearance preferences", async ({
  page,
}, testInfo) => {
  await page.goto("/");

  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(page.getByText("Post", { exact: true })).toBeVisible();
  await expect(page.getByText("Vespertine", { exact: true })).toBeVisible();

  const preferences =
    testInfo.project.name === "mobile"
      ? page.locator(".mobile-header")
      : page.locator(".sidebar");
  const theme = preferences.getByLabel("Theme");
  await theme.selectOption({ index: 1 });
  const selectedTheme = await theme.inputValue();
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("waveflow.theme")))
    .toBe(selectedTheme);

  const language = preferences.getByLabel("Language");
  await language.selectOption("fr");
  await expect(page.getByRole("link", { name: "Recherche" }).first()).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("lang", "fr");

  if (testInfo.project.name === "mobile") {
    await expect(page.locator(".mobile-navigation")).toBeVisible();
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(1);
  } else {
    await expect(page.locator(".sidebar")).toBeVisible();
  }
});

test("offers a keyboard skip link and a localized not-found route", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await page.keyboard.press("Tab");
  const skip = page.getByRole("link", { name: "Skip to content" });
  await expect(skip).toBeFocused();
  await skip.press("Enter");
  await expect(page.locator("#main-content")).toBeFocused();

  await page.goto("/a-room-that-does-not-exist");
  await expect(
    page.getByRole("heading", { name: "This room is silent" }),
  ).toBeVisible();
  await expect(page.getByRole("link", { name: "Back to the library" })).toBeVisible();
});

test("has no automated WCAG A or AA violations", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  expect(results.violations).toEqual([]);
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
  slowAlbum = new Promise<void>((resolve) => {
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

  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  expect(results.violations).toEqual([]);
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
    if (url.pathname === "/api/v2/songs") {
      asked.push(url.searchParams.get("genre"));
    }
  });

  await page.goto("/genres");
  await expect(page.getByRole("heading", { name: "Genres" })).toBeVisible();
  await expect(page.getByText("412 tracks · 31 albums")).toBeVisible();

  let results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  expect(results.violations).toEqual([]);

  await page.getByRole("link", { name: "Art Pop" }).click();
  await expect(page.getByRole("heading", { name: "Art Pop" })).toBeVisible();
  await expect(page.locator(".songs tbody tr")).toHaveCount(2);
  expect(asked).toEqual(["Art Pop"]);

  results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  expect(results.violations).toEqual([]);
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
 * The player bar's modes. `shuffledOrder`, `advance` and `retreat` are unit
 * tested; this is the wiring — that the buttons reach them, and that repeat
 * says which of its three states is on rather than only naming the action. A
 * screen reader hearing "repeat" alone could not tell.
 *
 * No audio is served: the bar renders from the restored queue, which is enough
 * to press its controls.
 */
test("carries shuffle and the three repeat states in the player bar", async ({
  page,
}, testInfo) => {
  await page.goto("/");
  const shuffle = page.getByRole("button", { name: "Shuffle" });

  if (testInfo.project.name === "mobile") {
    // Deliberate: a narrow bar keeps the transport and drops the modes. The
    // layout that came before dropped previous and next instead and kept only
    // play, which is the worse trade on a phone.
    await expect(shuffle).toBeHidden();
    await expect(page.getByRole("button", { name: "Repeat: off" })).toBeHidden();
    await expect(page.getByRole("button", { name: "Next" })).toBeVisible();
    return;
  }

  await expect(shuffle).toBeVisible();
  await expect(shuffle).toHaveAttribute("aria-pressed", "false");
  await shuffle.click();
  await expect(shuffle).toHaveAttribute("aria-pressed", "true");
  await shuffle.click();
  await expect(shuffle).toHaveAttribute("aria-pressed", "false");

  // Off → whole queue → this track → off.
  const repeat = page.getByRole("button", { name: "Repeat: off" });
  await repeat.click();
  await expect(
    page.getByRole("button", { name: "Repeat: the whole queue" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Repeat: the whole queue" }).click();
  await expect(
    page.getByRole("button", { name: "Repeat: this track" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Repeat: this track" }).click();
  await expect(page.getByRole("button", { name: "Repeat: off" })).toBeVisible();

  // The cover is the way back to the album that is playing.
  await expect(
    page.getByRole("link", { name: "Open the album: Vespertine" }),
  ).toHaveAttribute("href", "/albums/album-2");
});
