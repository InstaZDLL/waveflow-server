import { expect, type Page, test } from "@playwright/test";

import { expectNoWcagViolations } from "./fixtures/accessibility";
import { albums, session } from "./fixtures/catalogue";
import { PIXEL } from "./fixtures/media";

/**
 * The editorial pass over four screens: the playing screen in two columns,
 * the mixes set apart from the albums, the artist's image actions behind one
 * menu, and the measured bit depth said as a measurement.
 *
 * Its own mock, shaped like the one `fixtures/api.ts` serves — the
 * same session, the same `Library` shape, the same lyrics envelope — and kept
 * to the routes these screens read. Anything else answers 404, as there.
 */

const library = (role: "owner" | "manager" | "listener") => ({
  id: "library-1",
  name: "Ma musique",
  visibility: "private" as const,
  role,
  last_scan_started_at: null,
  last_scan_completed_at: null,
  accepts_uploads: false,
  accepts_canvas: false,
  follow_links: false,
});

const song = (index: number, title: string) => ({
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
  starred_at: null,
  user_rating: null,
  codec: "FLAC",
  suffix: "flac",
  size: 27_983_872,
  bitrate: 957,
  sample_rate: 96_000,
  channels: 2,
  bit_depth: 24,
});

const songs = [song(1, "Hidden Place"), song(2, "Cocoon"), song(3, "Undo")];

type Sheet = {
  displayArtist: string | null;
  displayTitle: string;
  lang: string;
  synced: boolean;
  line: Array<{ start?: number; value: string }>;
};

/** The words the mock holds per track; absent is a track without any. */
let lyricSheets = new Map<string, Sheet[]>();
/** The mixes `GET /api/v2/mixes` answers; none by default. */
let mixes: Array<{
  id: string;
  kind: string;
  genre: string | null;
  decade: number | null;
  track_count: number;
  artwork_hashes: string[];
}> = [];
/** The account's role in the one library. */
let role: "owner" | "manager" | "listener" = "owner";

/** A synced sheet of forty lines, one every five seconds. */
const longSheet = (): Sheet[] => [
  {
    displayArtist: "Björk",
    displayTitle: "Hidden Place",
    lang: "eng",
    synced: true,
    line: Array.from({ length: 40 }, (_, index) => ({
      start: index * 5_000,
      value: `column line ${index}`,
    })),
  },
];

async function mockApi(page: Page) {
  await page.context().addCookies([
    {
      name: "waveflow-csrf",
      value: "e2e-csrf",
      url: test.info().project.use.baseURL ?? "http://127.0.0.1:4173",
    },
  ]);
  await page.route("**/api/v2/**", async (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname;
    const answer = (json: unknown) => route.fulfill({ json });
    if (path === "/api/v2/web/auth/refresh") return answer(session);
    if (path === "/api/v2/libraries") return answer([library(role)]);
    if (path === "/api/v2/queue") {
      return answer({
        current: "song-1",
        position_ms: 0,
        changed_by: null,
        updated_at: 1,
        songs,
      });
    }
    if (path.startsWith("/api/v2/artwork/")) {
      return route.fulfill({ contentType: "image/png", body: PIXEL });
    }
    if (
      path === "/api/v2/bookmarks" ||
      path === "/api/v2/now-playing" ||
      path === "/api/v2/playlists" ||
      path === "/api/v2/smart-playlists" ||
      path === "/api/v2/radio-stations"
    ) {
      return answer([]);
    }
    if (path === "/api/v2/podcasts") {
      return answer({ enabled: false, can_manage: false, channels: [] });
    }
    if (path === "/api/v2/albums") return answer(albums);
    if (path === "/api/v2/albums/album-2")
      return answer({ ...albums[1], songs });
    if (path === "/api/v2/mixes") return answer(mixes);
    if (path === "/api/v2/artists/artist-1/top-songs") return answer([]);
    if (path === "/api/v2/artists/artist-1") {
      return answer({
        id: "artist-1",
        name: "Björk",
        album_count: 2,
        library_id: "library-1",
        artwork_hash: "d".repeat(64),
        banner_hash: "c".repeat(64),
        image_choice: {
          portrait: { source: "upload", set_at: 1 },
          banner: { source: "upload", set_at: 1 },
        },
        albums,
      });
    }
    const parts = path.split("/");
    if (parts[3] === "tracks" && parts.length === 6) {
      const trackId = parts[4] as string;
      if (parts[5] === "lyrics") {
        return answer({
          trackId,
          structuredLyrics: lyricSheets.get(trackId) ?? [],
        });
      }
      if (parts[5] === "quality") {
        // A 24-bit file whose samples use all 24: the case most easily read
        // as a certificate of genuine hi-res, which it is not.
        return answer({
          verdict: "consistent",
          cutoff_hz: null,
          wall_db: 4.1,
          nyquist_hz: 48_000,
          effective_bits: 24,
          analyzed_at: 1_700_000_000_000,
        });
      }
    }
    if (parts[3] === "tracks" && parts.length === 5) {
      const found = songs.find((entry) => entry.id === parts[4]);
      if (found) {
        return answer({
          ...found,
          relative_path: `Björk/Vespertine/${found.track}.flac`,
          full_hash: "a".repeat(64),
          created_at: 1_700_000_000_000,
        });
      }
    }
    return route.fulfill({ status: 404, json: { error: "not found" } });
  });
}

test.beforeEach(async ({ page }) => {
  lyricSheets = new Map();
  mixes = [];
  role = "owner";
  await mockApi(page);
});

/** Two boxes, read in one go so they describe the same layout. */
async function boxes(page: Page, first: string, second: string) {
  return page.evaluate(
    ([a, b]) =>
      [a, b].map((selector) => {
        const box = document.querySelector(selector)?.getBoundingClientRect();
        return box
          ? {
              left: box.left,
              right: box.right,
              top: box.top,
              bottom: box.bottom,
            }
          : null;
      }),
    [first, second] as const,
  );
}

/**
 * Wide, the track and its words are two columns side by side; at a phone's
 * width they stack, the track above. The cover grows in the wide layout: the
 * point of giving the track a column was the room.
 */
test("lays the playing screen out in two columns, and stacks it on a phone", async ({
  page,
}) => {
  lyricSheets.set("song-1", longSheet());
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/playing");
  const playing = page.locator(".playing");
  await expect(playing).toHaveClass(/playing-split/);
  await expect(page.locator(".playing-lyrics .lyrics ol")).toBeVisible();

  const [track, words] = await boxes(page, ".playing-track", ".playing-lyrics");
  if (!track || !words) throw new Error("a column is missing");
  // Beside, not under: the words start right of the track, level with it.
  expect(words.left).toBeGreaterThanOrEqual(track.right);
  expect(Math.abs(words.top - track.top)).toBeLessThanOrEqual(2);
  // Larger than the 18rem the cover was capped at beside the names.
  const cover = await page.locator(".playing-track .cover.large").boundingBox();
  expect(cover?.width ?? 0).toBeGreaterThan(18 * 16);
  await expectNoWcagViolations(page);

  await page.setViewportSize({ width: 390, height: 844 });
  await expect
    .poll(async () => {
      const [stackedTrack, stackedWords] = await boxes(
        page,
        ".playing-track",
        ".playing-lyrics",
      );
      return (
        stackedTrack !== null &&
        stackedWords !== null &&
        stackedWords.top >= stackedTrack.bottom
      );
    })
    .toBe(true);
  // And nothing wider than the phone.
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth - window.innerWidth,
    ),
  ).toBeLessThanOrEqual(1);
});

/**
 * The tall column still reads along: the sung line is marked and held in
 * view, the column scrolls itself and not the page, and a line pressed moves
 * the head to it.
 */
test("follows the sung line and seeks from the lyrics column", async ({
  page,
}) => {
  lyricSheets.set("song-1", longSheet());
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/playing");
  const column = page.locator(".playing-lyrics .lyrics");
  const list = column.locator("ol");
  await expect(list).toBeVisible();
  // Tall: more than the 26rem the strip under the names was given.
  expect(await list.evaluate((node) => node.clientHeight)).toBeGreaterThan(
    26 * 16,
  );
  await expect.poll(() => list.evaluate((node) => node.scrollTop)).toBe(0);

  for (let press = 0; press < 20; press += 1) {
    await page.keyboard.press("ArrowRight");
  }
  await expect(
    column.getByRole("listitem").filter({ hasText: /^column line 20$/ }),
  ).toHaveAttribute("aria-current", "true");
  await expect
    .poll(() => list.evaluate((node) => node.scrollTop))
    .toBeGreaterThan(0);
  expect(await page.evaluate(() => window.scrollY)).toBe(0);

  await column
    .getByRole("button", { name: "column line 5, jump to 0:25", exact: true })
    .click();
  await expect(
    column.getByRole("listitem").filter({ hasText: /^column line 5$/ }),
  ).toHaveAttribute("aria-current", "true");
  await expectNoWcagViolations(page);
});

/**
 * A track without words keeps one column, in the middle, and says there is
 * nothing to read — rather than holding half the screen open for words that
 * are not there.
 */
test("keeps one centred column for a track without lyrics", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/playing");
  const playing = page.locator(".playing");
  await expect(
    page.getByText("No lyrics travel with this file."),
  ).toBeVisible();
  await expect(playing).toHaveClass(/playing-solo/);
  await expect(page.locator(".playing-lyrics")).toHaveCount(0);

  const [frame, track] = await boxes(page, ".playing", ".playing-track");
  if (!frame || !track) throw new Error("the layout is missing");
  const middle = (box: { left: number; right: number }) =>
    (box.left + box.right) / 2;
  expect(Math.abs(middle(track) - middle(frame))).toBeLessThanOrEqual(2);
  await expectNoWcagViolations(page);
});

/**
 * The mixes are a shelf of their own, named as generated playlists, each card
 * marked as a mix and the one with four covers drawn as a mosaic; the albums
 * under it carry their own heading, and take no card from the shelf.
 */
test("sets the day's mixes apart from the albums", async ({ page }) => {
  mixes = [
    {
      id: "most-played",
      kind: "most_played",
      genre: null,
      decade: null,
      track_count: 2,
      artwork_hashes: ["1", "2", "3", "4"].map((digit) => digit.repeat(64)),
    },
    {
      id: "genre:art pop",
      kind: "genre",
      genre: "Art Pop",
      decade: null,
      track_count: 2,
      artwork_hashes: ["5".repeat(64)],
    },
  ];
  await page.goto("/");
  const shelf = page.getByRole("region", { name: "Mixes for you" });
  await expect(shelf).toBeVisible();
  await expect(
    shelf.getByText("Playlists drawn from your listening, new every day."),
  ).toBeVisible();
  // Each card says it is a mix, in words.
  const cards = shelf.getByRole("link");
  await expect(cards).toHaveCount(2);
  for (const card of await cards.all()) {
    await expect(card.locator(".mix-badge")).toHaveText("Mix");
  }
  await expect(cards.first()).toHaveAccessibleName(/Mix.*Most played/);
  // Four sleeves in a square for the mix that has four; one for the other.
  await expect(cards.first().locator(".mix-mosaic img")).toHaveCount(4);
  await expect(cards.nth(1).locator(".mix-mosaic")).toHaveCount(0);

  // The albums carry a heading of their own, under the shelf, and the shelf's
  // cards are not among them.
  const albumsHeading = page.getByRole("heading", { name: "In your library" });
  await expect(albumsHeading).toBeVisible();
  const order = await page.evaluate(() => {
    const top = (selector: string) =>
      document.querySelector(selector)?.getBoundingClientRect().top ?? -1;
    return [top(".mix-shelf"), top(".albums-grid-title"), top(".grid")];
  });
  expect(order[0]).toBeLessThan(order[1] as number);
  expect(order[1]).toBeLessThan(order[2] as number);
  await expect(page.locator(".grid li")).toHaveCount(2);
  await expect(page.locator(".grid .mix-badge")).toHaveCount(0);
  await expectNoWcagViolations(page);

  // Filtering the albums puts the shelf away, and the heading with it.
  await page.getByLabel("Filter by title or artist").fill("Post");
  await expect(shelf).toHaveCount(0);
  await expect(albumsHeading).toHaveCount(0);
});

test("names no album section when there is no shelf above it", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(page.locator(".grid li")).toHaveCount(2);
  await expect(
    page.getByRole("heading", { name: "In your library" }),
  ).toHaveCount(0);
});

/**
 * The image actions sit behind one button on the header, for whoever may
 * take them: a menu button that says it opens a menu and whether it is open,
 * whose menu answers the keyboard and hands focus back when it closes.
 */
test("offers an editor the artist's image actions behind one menu", async ({
  page,
}) => {
  await page.goto("/artists/artist-1");
  const hero = page.locator(".artist-hero");
  await expect(hero.locator("img.artist-banner")).toBeVisible();
  const trigger = hero.getByRole("button", {
    name: "Edit images",
    exact: true,
  });
  await expect(trigger).toBeVisible();
  await expect(trigger).toHaveAttribute("aria-haspopup", "menu");
  await expect(trigger).toHaveAttribute("aria-expanded", "false");
  // None of the actions is a button on the picture any more.
  for (const name of [
    "Change portrait",
    "Search Deezer",
    "Automatic portrait",
    "Change banner",
  ]) {
    await expect(hero.getByRole("button", { name, exact: true })).toHaveCount(
      0,
    );
  }
  await expectNoWcagViolations(page);

  await trigger.click();
  await expect(trigger).toHaveAttribute("aria-expanded", "true");
  const menu = page.getByRole("menu", { name: "Images of Björk" });
  // The portrait chosen by hand and the banner too: both can go back.
  await expect(menu.getByRole("menuitem")).toHaveText([
    "Change portrait",
    "Search Deezer",
    "Automatic portrait",
    "Change banner",
    "No banner",
  ]);
  await expect(menu.getByRole("menuitem").first()).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(
    menu.getByRole("menuitem", { name: "Search Deezer" }),
  ).toBeFocused();
  await expectNoWcagViolations(page);

  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);
  await expect(trigger).toBeFocused();
  await expect(trigger).toHaveAttribute("aria-expanded", "false");

  // The button closes what it opened, rather than closing and reopening it.
  await trigger.click();
  await expect(menu).toBeVisible();
  await trigger.click();
  await expect(menu).toHaveCount(0);
});

test("shows a reader the artist's header with no edit menu", async ({
  page,
}) => {
  role = "listener";
  await page.goto("/artists/artist-1");
  const hero = page.locator(".artist-hero");
  await expect(hero.locator("img.artist-banner")).toBeVisible();
  await expect(hero.getByRole("heading", { name: "Björk" })).toBeVisible();
  await expect(hero.getByRole("button", { name: "Edit images" })).toHaveCount(
    0,
  );
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expectNoWcagViolations(page);
});

/**
 * The measured bit depth is a measurement, and says so: a file that uses all
 * 24 bits it declares was not padded, which does not make it a genuine
 * hi-res recording.
 */
test("presents the measured bit depth as a measurement", async ({ page }) => {
  await page.goto("/albums/album-2");
  await page
    .locator(".songs tbody tr")
    .first()
    .locator("button.song-title")
    .focus();
  await page.keyboard.press("Shift+F10");
  await page.getByRole("menuitem", { name: "Properties" }).click();
  const dialog = page.getByRole("dialog");
  await expect(
    dialog.getByRole("heading", { name: "Real quality" }),
  ).toBeVisible();
  const row = dialog
    .locator(".property-row")
    .filter({ hasText: "Bits in use, measured" });
  await expect(row.locator("dd")).toHaveText("24 of 24");
  await expect(dialog.locator(".quality-note")).toHaveText(
    "A measurement of the samples, not proof of genuine hi-res: a file that uses every bit it declares was not padded, which does not mean it was recorded at that depth.",
  );
  await expectNoWcagViolations(page);
});
