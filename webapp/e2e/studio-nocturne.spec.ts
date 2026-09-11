import AxeBuilder from "@axe-core/playwright";
import { blake3 } from "@noble/hashes/blake3.js";
import { bytesToHex } from "@noble/hashes/utils.js";
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

/**
 * The whole `Library` shape, not just what the picker reads. A fixture that
 * only carries the fields one screen happens to use is how the admin page went
 * down on an absent `folder_ids`: the next screen to touch it reads a field
 * nobody remembered to mock.
 */
const library = (id: string, name: string) => ({
  id,
  name,
  visibility: "private" as const,
  role: "owner" as "owner" | "manager" | "listener",
  last_scan_started_at: null,
  last_scan_completed_at: null,
  accepts_uploads: false as boolean,
});

/** Replaced by the scoping test; one library elsewhere, so no picker appears. */
let libraries: Array<ReturnType<typeof library>> = [library("library-1", "Ma musique")];

/** Flipped by the two tests that check a failure is shown as one. */
let membersFail = false;
let tokensFail = false;
let scanFails = false;
/** The stream never answers at all, which is what a lost network looks like. */
let scanDrops = false;

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

/**
 * A track another client has already corrected: a comment over two lines, and
 * an artist fixed. A save that does not touch the comment must leave it out of
 * the body — the server keeps whatever a patch does not mention, but only if
 * the client does not send it back.
 *
 * Built afresh for each test, because the PATCH mock applies what it receives.
 * The editor reads the corrections back after a save, and a mock that ignored
 * the write would let a form that never reloaded pass for one that did.
 */
function correctableTrack() {
  const overrides: Record<string, unknown> = {
    title: null,
    sort_title: null,
    year: null,
    track_number: null,
    disc_number: null,
    musicbrainz_recording_id: null,
    comment: "Remastered\nBonus edition",
    artists: ["Skunk Anansie"],
    genres: null,
  };
  return {
    song: {
      ...song(9, "Army Of Me", 0, false),
      year: 1995,
      artists: [{ id: "artist-2", name: "Skunk Anansie" }],
      genres: ["Trip Hop"],
    },
    tracked: {
      source: {
        title: "Army Of Me",
        sort_title: null,
        year: 1995,
        track_number: 9,
        disc_number: 1,
        musicbrainz_recording_id: null,
        comment: null,
      },
      overrides,
    },
  };
}

/** What the file credits: where a removed artists correction goes back to. */
const fileArtists = [{ id: "artist-1", name: "Björk" }];

let correctable = correctableTrack();

/** Every correction body sent, in order. */
let corrections: unknown[] = [];

type Offer = { full_hash: string; size_bytes: number; extension: string };

const freshUploadSession = () => ({
  session_id: "upload-1",
  next_chunk: 0,
  received_bytes: 0,
  chunk_bytes: 8,
  expires_at: 0,
});

/** What the upload mock received, in order. */
let uploadOffers: Offer[] = [];
let uploadChunks: Array<{ index: number; bytes: number[] }> = [];
let uploadCommits: string[] = [];
/** The one session the mock holds, advanced by each fragment it writes. */
let uploadSession = freshUploadSession();
/**
 * Set by the resume test: the fragment at this index is written, and its
 * acknowledgement lost — the session advances and the client is told 409.
 */
let loseAcknowledgementOf: number | null = null;

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
    if (url.pathname === "/api/v2/scans/scan-1/events" && scanDrops) {
      await route.abort("connectionfailed");
      return;
    }
    if (url.pathname === "/api/v2/scans/scan-1/events" && scanFails) {
      await route.fulfill({ status: 503, body: "" });
      return;
    }
    if (url.pathname === "/api/v2/scans/scan-1/events") {
      // Two frames, deliberately split so the client has to hold a partial
      // one between reads — which is what a real stream does.
      const job = (processed: number, status: string) =>
        JSON.stringify({
          id: "scan-1",
          library_id: "library-1",
          status,
          total_files: 10,
          processed_files: processed,
          added: processed,
          updated: 0,
          moved: 0,
          skipped: 0,
          unavailable: 0,
          errors: 0,
          current_path: `/music/track-${processed}.flac`,
          message: null,
        });
      await route.fulfill({
        status: 200,
        headers: { "content-type": "text/event-stream" },
        body:
          `event: snapshot\ndata: ${job(3, "running")}\n\n` +
          `: keep-alive\n\n` +
          `event: progress\ndata: ${job(10, "completed")}\n\n`,
      });
      return;
    }
    if (url.pathname.endsWith("/scans")) {
      await route.fulfill({ json: { scan_id: "scan-1" } });
      return;
    }
    if (url.pathname === "/api/v2/admin/users") {
      await route.fulfill({
        json: [
          {
            id: "user-1",
            username: "listener",
            role: "admin",
            disabled: false,
            has_subsonic_credential: false,
            folder_ids: ["library-1"],
          },
        ],
      });
      return;
    }
    if (url.pathname.endsWith("/members") && membersFail) {
      await route.fulfill({ status: 500, json: { error: "boom" } });
      return;
    }
    if (url.pathname.endsWith("/members")) {
      await route.fulfill({
        json: [
          {
            user_id: "user-1",
            username: "listener",
            role: "owner",
            created_at: 1,
          },
          {
            user_id: "user-2",
            username: "guest",
            role: "listener",
            created_at: 2,
          },
        ],
      });
      return;
    }
    if (url.pathname.endsWith("/tokens")) {
      if (tokensFail) {
        await route.fulfill({ status: 500, json: { error: "boom" } });
        return;
      }
      await route.fulfill({ json: [] });
      return;
    }
    if (url.pathname === "/api/v2/now-playing") {
      await route.fulfill({ json: [] });
      return;
    }
    if (url.pathname === "/api/v2/libraries") {
      await route.fulfill({ json: libraries });
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
    if (url.pathname === "/api/v2/libraries/library-1/uploads") {
      const { offers } = route.request().postDataJSON() as { offers: Offer[] };
      uploadOffers.push(...offers);
      await route.fulfill({
        json: {
          verdicts: offers.map((offer) => ({
            full_hash: offer.full_hash,
            decision: "accepted",
            session: uploadSession,
          })),
        },
      });
      return;
    }
    if (url.pathname === "/api/v2/uploads/upload-1") {
      await route.fulfill({ json: uploadSession });
      return;
    }
    if (url.pathname.startsWith("/api/v2/uploads/upload-1/chunks/")) {
      const index = Number(url.pathname.split("/").pop());
      if (index !== uploadSession.next_chunk) {
        await route.fulfill({ status: 409, json: { code: "conflict" } });
        return;
      }
      const body = route.request().postDataBuffer() ?? Buffer.alloc(0);
      uploadChunks.push({ index, bytes: [...body] });
      uploadSession = {
        ...uploadSession,
        next_chunk: index + 1,
        received_bytes: uploadSession.received_bytes + body.length,
      };
      if (index === loseAcknowledgementOf) {
        loseAcknowledgementOf = null;
        await route.fulfill({ status: 409, json: { code: "conflict" } });
        return;
      }
      await route.fulfill({ json: uploadSession });
      return;
    }
    if (url.pathname === "/api/v2/uploads/upload-1/commit") {
      uploadCommits.push("upload-1");
      await route.fulfill({
        status: 201,
        json: { track_id: "song-uploaded", full_hash: uploadOffers[0]?.full_hash },
      });
      return;
    }
    if (url.pathname === "/api/v2/tracks/song-9/overrides") {
      await route.fulfill({ json: correctable.tracked });
      return;
    }
    if (url.pathname === "/api/v2/tracks/song-9") {
      if (route.request().method() === "PATCH") {
        const body = route.request().postDataJSON() as Record<string, unknown>;
        corrections.push(body);
        // What the server does with it: a key present replaces its correction,
        // null removes it, and a list that is removed goes back to the file.
        Object.assign(correctable.tracked.overrides, body);
        if (typeof body.title === "string") correctable.song.title = body.title;
        if (body.artists === null) correctable.song.artists = fileArtists;
      }
      await route.fulfill({ json: correctable.song });
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
    if (url.pathname === "/api/v2/songs/by-genre") {
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
  libraries = [library("library-1", "Ma musique")];
  membersFail = false;
  tokensFail = false;
  scanFails = false;
  scanDrops = false;
  corrections = [];
  correctable = correctableTrack();
  uploadOffers = [];
  uploadChunks = [];
  uploadCommits = [];
  uploadSession = freshUploadSession();
  loseAcknowledgementOf = null;
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
    if (url.pathname === "/api/v2/songs/by-genre") {
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

  libraries = [
    library("library-1", "Ma musique"),
    library("library-2", "Les enfants"),
  ];

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();

  // The picker exists in the sidebar and in the mobile header; only one of the
  // two is on screen, so the locator has to say which — as the appearance test
  // above already does for theme and language.
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

/**
 * Live scan progress. The stream is read with `fetch` and not `EventSource`,
 * because the route authenticates on a bearer token and `EventSource` sends no
 * headers — the same wall `<audio src>` hits. That puts the framing on the
 * client, so this covers a stream that arrives in pieces with a keep-alive
 * comment in the middle of it.
 */
test("follows a scan while it runs", async ({ page }) => {
  await page.goto("/admin");
  await expect(
    page.getByRole("heading", { name: "Administration" }),
  ).toBeVisible();

  await page.getByRole("button", { name: "Scan now" }).first().click();

  const panel = page.locator(".scan-panel");
  await expect(panel).toBeVisible();
  // The snapshot lands first, then the progress frame replaces it whole.
  await expect(panel.getByText("10 of 10 files")).toBeVisible();
  await expect(panel.getByText("Finished")).toBeVisible();
  await expect(
    panel.getByText("10 added · 0 updated · 0 moved · 0 unchanged · 0 errors"),
  ).toBeVisible();
  // A finished scan stops showing the file it is on.
  await expect(panel.locator(".scan-path")).toHaveCount(0);
});

/**
 * API tokens are rendered once per account, so loading them on mount meant one
 * request per account every time the admin screen opened — for a list almost
 * nobody opens. They wait for the panel to be opened.
 */
test("asks for a account's tokens only when its panel is opened", async ({
  page,
}) => {
  const asked: string[] = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (path.endsWith("/tokens")) asked.push(path);
  });

  await page.goto("/admin");
  await expect(
    page.getByRole("heading", { name: "Administration" }),
  ).toBeVisible();
  const disclosure = page.getByRole("button", {
    name: "API tokens for listener",
  });
  await expect(disclosure).toHaveAttribute("aria-expanded", "false");
  expect(asked).toEqual([]);

  await disclosure.click();
  await expect(disclosure).toHaveAttribute("aria-expanded", "true");
  await expect(page.getByText("No token on this account.")).toBeVisible();
  expect(asked).toEqual(["/api/v2/admin/users/listener/tokens"]);
});

/**
 * Both of these were failures wearing the clothes of an ordinary state: a
 * token list that could not be fetched read as "no token on this account", and
 * a progress stream that could not open left the panel waiting for a first
 * reading that was never coming. That is the kind of defect nobody reports,
 * because the screen looks like it is working.
 */
test("says a token list failed instead of calling it empty", async ({
  page,
}) => {
  tokensFail = true;
  await page.goto("/admin");
  await page
    .getByRole("button", { name: "API tokens for listener" })
    .click();

  // Scoped to the panel: the player bar raises an alert of its own here, its
  // stream being mocked away.
  await expect(page.locator(".token-panel").getByRole("alert")).toHaveText(
    "We could not load this view",
  );
  await expect(page.getByText("No token on this account.")).toHaveCount(0);
});

test("says the progress stream was lost instead of waiting for ever", async ({
  page,
}) => {
  scanFails = true;
  await page.goto("/admin");
  await page.getByRole("button", { name: "Scan now" }).first().click();

  const panel = page.locator(".scan-panel");
  await expect(panel.getByRole("alert")).toContainText(
    "The progress stream could not be opened",
  );
  await expect(panel.getByText("Waiting for the first reading")).toHaveCount(0);
});

/**
 * A refused status is the tidy failure. The likelier one is the request never
 * completing at all, and that arrives as a rejected `fetch` rather than a
 * response — which the stream reader used to swallow whole, leaving the panel
 * waiting on a reading that had no chance of coming.
 */
test("says so when the progress stream never connects", async ({ page }) => {
  scanDrops = true;
  await page.goto("/admin");
  await page.getByRole("button", { name: "Scan now" }).first().click();

  const panel = page.locator(".scan-panel");
  await expect(panel.getByRole("alert")).toContainText(
    "The progress stream could not be opened",
  );
});

/**
 * Library membership could be written since M4 and never read, so an interface
 * could grant and revoke without ever showing who already had access. The list
 * is behind a disclosure for the same reason the tokens are: the panel renders
 * once per library, and loading on mount would ask for every membership list
 * every time the admin screen opened.
 */
test("lists who may see a library, once its panel is opened", async ({
  page,
}) => {
  const asked: string[] = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (path.endsWith("/members")) asked.push(path);
  });

  // Two libraries, because one cannot tell the two layouts apart: with a single
  // library, "list then panel" and "list, then every panel" produce the same
  // rows. The defect only shows from the second library on.
  libraries = [
    library("library-1", "Ma musique"),
    library("library-2", "Les enfants"),
  ];

  await page.goto("/admin");
  const disclosure = page
    .getByRole("button", { name: "Who may see this library" })
    .first();
  await expect(disclosure).toHaveAttribute("aria-expanded", "false");
  expect(asked).toEqual([]);

  // Each library's membership sits directly under that library, not after the
  // whole list: two render passes put the third library's members four rows
  // below it, which reads as belonging to whatever is above them.
  const rows = page.locator(".admin-panel .resource-list > li");
  await expect(rows.nth(0)).toContainText("Ma musique");
  await expect(rows.nth(1)).toContainText("Who may see this library");
  await expect(rows.nth(2)).toContainText("Les enfants");
  await expect(rows.nth(3)).toContainText("Who may see this library");

  await disclosure.click();
  await expect(page.getByText("guest")).toBeVisible();
  expect(asked).toEqual(["/api/v2/libraries/library-1/members"]);

  // The owner is shown and offers no role control: the route refuses `owner`
  // outright, so a select here would be offering a refusal.
  await expect(page.getByText("owner, and stays one")).toBeVisible();
  await expect(
    page.getByRole("combobox", { name: "Role: listener" }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("combobox", { name: "Role: guest" }),
  ).toHaveValue("listener");
});

/**
 * A membership list that could not be read is not an empty one. Standing an
 * empty array in for "not known yet" made every account on the server look like
 * a non-member, so the panel offered access to people who already had it —
 * printed underneath the notice saying the list could not be read.
 */
test("offers no membership to grant while the list is unknown", async ({
  page,
}) => {
  membersFail = true;
  await page.goto("/admin");
  await page
    .getByRole("button", { name: "Who may see this library" })
    .first()
    .click();

  const panel = page.locator(".member-row .admin-panel").first();
  await expect(panel.getByRole("alert")).toHaveText(
    "We could not load this view",
  );
  // The grant control is built from the list, so without one there is nothing
  // to build it from.
  await expect(panel.getByLabel("Give access to")).toHaveCount(0);
});

/**
 * The editor's whole job, seen from the wire. The body must name the field that
 * changed and the list handed back to the file, and nothing else — above all
 * not the comment another client added, which a body sending the whole form
 * would have erased before #177 and would pin now.
 */
test("corrects a track's tags and sends only what changed", async ({
  page,
}) => {
  await page.goto("/tracks/song-9/edit");
  await expect(
    page.getByRole("heading", { name: "Correct tags" }),
  ).toBeVisible();

  const title = page.getByLabel("Title", { exact: true });
  await expect(title).toHaveValue("Army Of Me");
  // Provenance is shown here and nowhere else: the corrected value, and what
  // the file says beneath it.
  // A tagger's comment keeps both of its lines: an `<input>` would flatten it.
  await expect(page.getByLabel("Comment", { exact: true })).toHaveValue(
    "Remastered\nBonus edition",
  );
  await expect(page.getByText("The file says nothing here")).toBeVisible();

  await title.fill("Army of Me");
  await page
    .getByRole("button", { name: /^Restore the file.s value: Artists$/ })
    .click();
  await expect(page.getByLabel("Artists", { exact: true })).toBeDisabled();

  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByText("Saved.")).toBeVisible();
  expect(corrections).toEqual([{ title: "Army of Me", artists: null }]);

  // Read back, not assumed. The form now stands on the corrections the server
  // holds: the artists are the file's again, and no longer corrected.
  await expect(title).toHaveValue("Army of Me");
  const artists = page.getByLabel("Artists", { exact: true });
  await expect(artists).toBeEnabled();
  await expect(artists).toHaveValue("Björk");
  await expect(
    page.getByRole("button", { name: /^Restore the file.s value: Artists$/ }),
  ).toHaveCount(0);

  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  expect(results.violations).toEqual([]);
});

/**
 * Offered where the server would accept it, and nowhere else. The listener half
 * is not vacuous: the album waits for the list of libraries before it renders,
 * so the rows below exist only once the role that decides them is known.
 */
test("offers a tag correction only to an owner or a manager", async ({
  page,
}) => {
  await page.goto("/albums/album-2");
  await expect(page.getByRole("heading", { name: "Vespertine" })).toBeVisible();
  const links = page.getByRole("link", { name: /^Correct tags: / });
  await expect(links).toHaveCount(3);
  await expect(links.first()).toHaveAttribute("href", "/tracks/song-1/edit");

  // A manager may correct as well: the other half of the pair the server
  // names, and the half a regression to owner-only would drop.
  libraries = [{ ...library("library-1", "Ma musique"), role: "manager" }];
  await page.reload();
  await expect(page.getByRole("heading", { name: "Vespertine" })).toBeVisible();
  await expect(links).toHaveCount(3);

  libraries = [{ ...library("library-1", "Ma musique"), role: "listener" }];
  await page.reload();
  await expect(page.getByRole("heading", { name: "Vespertine" })).toBeVisible();
  // The row's own marks cell, where the link would be. Once its rating is in
  // the page, the absence of the link is an answer and not a race.
  await expect(
    page.getByRole("group", { name: "Rating: Hidden Place" }),
  ).toBeAttached();
  await expect(links).toHaveCount(0);
});

/**
 * A file goes through the three steps RFC-008 describes, and the wire carries
 * what the server checks: the whole file's BLAKE3 as the offer, fragments of
 * exactly the advertised size in order, and one commit.
 *
 * The acknowledgement of the second fragment is lost on purpose. The client
 * must read the session back and carry on from where the server stands: not
 * send the written fragment again, and not skip the next one.
 */
test("uploads a file in fragments, resuming from the server's account", async ({
  page,
}) => {
  libraries = [{ ...library("library-1", "Ma musique"), accepts_uploads: true }];
  loseAcknowledgementOf = 1;
  const bytes = Buffer.from(Array.from({ length: 20 }, (_, i) => i));

  await page.goto("/upload");
  await expect(
    page.getByRole("heading", { name: "Upload to Ma musique" }),
  ).toBeVisible();
  await page.getByLabel("Choose audio files").setInputFiles({
    name: "Army of Me.flac",
    mimeType: "audio/flac",
    buffer: bytes,
  });
  await expect(page.getByText("Added to the library")).toBeVisible();

  // The browser's digest is the one the server recomputes, computed here by an
  // independent call rather than read back from the page.
  expect(uploadOffers).toEqual([
    { full_hash: bytesToHex(blake3(bytes)), size_bytes: 20, extension: "flac" },
  ]);
  expect(uploadChunks.map(({ index, bytes: sent }) => [index, sent.length])).toEqual([
    [0, 8],
    [1, 8],
    [2, 4],
  ]);
  expect(uploadChunks.flatMap(({ bytes: sent }) => sent)).toEqual([...bytes]);
  expect(uploadCommits).toEqual(["upload-1"]);

  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  expect(results.violations).toEqual([]);
});

/**
 * An extension the scanner does not index is refused on the spot. Hashing it
 * first would spend the time of a full read to be told `unsupported_format`.
 */
test("refuses a file the library cannot index, before hashing it", async ({
  page,
}) => {
  libraries = [{ ...library("library-1", "Ma musique"), accepts_uploads: true }];
  await page.goto("/upload");
  await page.getByLabel("Choose audio files").setInputFiles({
    name: "notes.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("not audio"),
  });
  await expect(
    page.getByText("Refused: not an audio format the library can index"),
  ).toBeVisible();
  expect(uploadOffers).toEqual([]);
});

/**
 * A worker that cannot load sends no message at all. Waiting for one would leave
 * the row on "Fingerprinting…" for ever, and the queue — one file at a time —
 * stuck behind it. Both files must fail, the second without waiting on the
 * first, and nothing may be offered to the server.
 */
test("fails a file whose fingerprinting worker cannot load, and moves on", async ({
  page,
}) => {
  libraries = [{ ...library("library-1", "Ma musique"), accepts_uploads: true }];
  await page.route("**/assets/hash-worker-*.js", (route) =>
    route.fulfill({ status: 404, body: "" }),
  );
  await page.goto("/upload");
  await page.getByLabel("Choose audio files").setInputFiles([
    { name: "first.flac", mimeType: "audio/flac", buffer: Buffer.from([1, 2, 3]) },
    { name: "second.flac", mimeType: "audio/flac", buffer: Buffer.from([4, 5, 6]) },
  ]);
  await expect(
    page.getByText("Failed: this browser could not fingerprint the file"),
  ).toHaveCount(2);
  expect(uploadOffers).toEqual([]);
});

/**
 * Two locks, and the link needs both: a role that may upload, and a library
 * its operator has opened. The page behind it says which one is missing when
 * reached directly.
 */
test("offers the upload only where the library takes files and the role may add them", async ({
  page,
}) => {
  // Counted in the DOM, not by role: a role query skips hidden elements, and
  // the sidebar is hidden on a phone — the count would be 0 there whether the
  // link exists or not, which makes the zero below prove nothing.
  const link = page.locator('.primary-navigation a[href="/upload"]');

  // The fixture's default: an owner, in a library closed to files.
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(link).toHaveCount(0);

  libraries = [{ ...library("library-1", "Ma musique"), accepts_uploads: true }];
  await page.reload();
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(link).toHaveCount(1);

  libraries = [
    {
      ...library("library-1", "Ma musique"),
      accepts_uploads: true,
      role: "listener",
    },
  ];
  await page.reload();
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(link).toHaveCount(0);
  await page.goto("/upload");
  await expect(
    page.getByText(
      "Only an owner or a manager of this library can add files to it.",
    ),
  ).toBeVisible();
});
