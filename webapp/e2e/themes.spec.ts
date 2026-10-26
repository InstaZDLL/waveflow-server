import AxeBuilder from "@axe-core/playwright";
import { expect, type Page, test } from "@playwright/test";

import { THEME_PRESETS } from "../src/styles/design-tokens/themes";

/**
 * The theme picker and the fourteen presets it offers, against a mock
 * server: the cards, the keyboard, following the device's scheme, and the
 * colour contrast of every preset on the screens that spend its colours most.
 *
 * The roster is read from the catalogue itself rather than copied here, so a
 * preset added to `THEME_PRESETS` is audited without anyone remembering to.
 */

const session = {
  access_token: "e2e-access-token",
  user: { id: "user-1", username: "listener", role: "admin" },
  device_id: "device-1",
};

const library = {
  id: "library-1",
  name: "Ma musique",
  visibility: "private",
  role: "owner",
  last_scan_started_at: null,
  last_scan_completed_at: null,
  accepts_uploads: false,
  accepts_canvas: false,
  follow_links: false,
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

const song = (index: number, title: string, rating: number, starred: boolean) => ({
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
  codec: "FLAC",
  suffix: "flac",
  size: 27_983_872,
  bitrate: 957,
  sample_rate: 44_100,
  channels: 2,
  bit_depth: 16,
});

/** Starred and rated, so the song table draws its accented marks too. */
const songs = [
  song(1, "Hidden Place", 5, true),
  song(2, "Cocoon", 0, false),
  song(3, "Undo", 2, false),
];

/** Thirty seconds of silence, as a WAV the browser can load. */
function silence(): Buffer {
  const samples = 8000 * 30;
  const header = Buffer.alloc(44);
  header.write("RIFF", 0);
  header.writeUInt32LE(36 + samples, 4);
  header.write("WAVE", 8);
  header.write("fmt ", 12);
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(1, 22);
  header.writeUInt32LE(8000, 24);
  header.writeUInt32LE(8000, 28);
  header.writeUInt16LE(1, 32);
  header.writeUInt16LE(8, 34);
  header.write("data", 36);
  header.writeUInt32LE(samples, 40);
  return Buffer.concat([header, Buffer.alloc(samples, 128)]);
}

async function mockApi(page: Page) {
  await page.context().addCookies([
    {
      name: "waveflow-csrf",
      value: "e2e-csrf",
      url: test.info().project.use.baseURL ?? "http://127.0.0.1:4173",
    },
  ]);
  await page.route("**/api/v2/**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    const answers: Record<string, unknown> = {
      "/api/v2/web/auth/refresh": session,
      "/api/v2/libraries": [library],
      "/api/v2/albums": albums,
      "/api/v2/albums/album-2": { ...albums[1], songs },
      // A queue the server remembers, so the play bar is drawn on every
      // screen and measured with it.
      "/api/v2/queue": {
        current: "song-1",
        position_ms: 0,
        changed_by: null,
        updated_at: 1,
        songs,
      },
      "/api/v2/playlists": [],
      "/api/v2/smart-playlists": [],
      "/api/v2/podcasts": { enabled: false, can_manage: false, channels: [] },
      "/api/v2/radio-stations": [],
      "/api/v2/now-playing": [],
      "/api/v2/bookmarks": [],
      "/api/v2/api-tokens": [],
      "/api/v2/subsonic-credential": null,
      "/api/v2/shares": [],
    };
    if (path in answers) {
      await route.fulfill({ json: answers[path] });
      return;
    }
    // The remembered track loads like a real one, so the play bar is drawn
    // in its ordinary state and not in its playback error.
    const ticket = path.match(/^\/api\/v2\/tracks\/([^/]+)\/stream-ticket$/);
    if (ticket) {
      await route.fulfill({
        json: {
          url: `/api/v2/stream/ticket-${ticket[1]}`,
          expires_at: Date.now() + 3_600_000,
        },
      });
      return;
    }
    if (path.startsWith("/api/v2/stream/")) {
      await route.fulfill({
        body: silence(),
        headers: { "content-type": "audio/wav" },
      });
      return;
    }
    if (path.startsWith("/api/v2/tracks/")) {
      await route.fulfill({ json: songs[0] });
      return;
    }
    await route.fulfill({ status: 404, json: { error: "not found" } });
  });
}

test.beforeEach(async ({ page }) => {
  await mockApi(page);
});

/** Lets every finite animation land, so nothing is measured mid-fade. */
async function settle(page: Page) {
  await page.evaluate(() =>
    Promise.all(
      document
        .getAnimations()
        .filter(
          (animation) =>
            animation.effect?.getComputedTiming().endTime !== Infinity,
        )
        .map((animation) => animation.finished.catch(() => undefined)),
    ),
  );
}

function picker(page: Page) {
  return page.getByRole("radiogroup", { name: "Theme" });
}

test("draws a card per preset under a Light and a Dark heading", async ({
  page,
}) => {
  await page.goto("/settings");
  const themes = picker(page);
  await expect(themes).toBeVisible();
  await expect(themes.getByRole("radio")).toHaveCount(14);
  await expect(
    themes.getByRole("group", { name: "Light" }).getByRole("radio"),
  ).toHaveCount(6);
  await expect(
    themes.getByRole("group", { name: "Dark" }).getByRole("radio"),
  ).toHaveCount(8);

  // A first visit keeps the default, and its card is the checked one, with
  // the accent's edge and the check that says so.
  await expect(page.locator("html")).toHaveAttribute("data-theme", "default-dark");
  const current = themes.getByRole("radio", { checked: true });
  await expect(current).toHaveCount(1);
  await expect(current).toHaveAccessibleName("Emerald dark");
  const card = page.locator('.theme-card[data-theme-id="default-dark"]');
  await expect(card.locator(".theme-card-check")).toBeVisible();
  await expect(page.locator(".theme-card-check")).toHaveCount(1);

  // Each miniature is painted in its own preset's colours, not the page's:
  // the Midnight card's sidebar is Midnight's surface while Emerald is on.
  const midnightSidebar = await page
    .locator('.theme-card[data-theme-id="midnight"] .theme-card-sidebar')
    .evaluate((element) => getComputedStyle(element).backgroundColor);
  const emeraldSidebar = await page
    .locator('.theme-card[data-theme-id="default-dark"] .theme-card-sidebar')
    .evaluate((element) => getComputedStyle(element).backgroundColor);
  expect(midnightSidebar).not.toBe(emeraldSidebar);

  // Picking a card applies and stores it.
  await themes.getByRole("radio", { name: "Midnight", exact: true }).check();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "midnight");
  await expect(
    themes.getByRole("radio", { name: "Midnight", exact: true }),
  ).toBeChecked();
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("waveflow.theme")))
    .toBe("midnight");
  await expect(
    page.locator('.theme-card[data-theme-id="midnight"] .theme-card-check'),
  ).toBeVisible();
  await expect(page.locator(".theme-card-check")).toHaveCount(1);
});

test("walks the cards with the arrow keys, selecting as it goes", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name === "mobile", "a keyboard test");
  await page.goto("/settings");
  const themes = picker(page);
  // One stop in the tab order, on the checked card: tabbing from the box
  // above lands on Emerald dark and not on the first card.
  await page.getByRole("checkbox", { name: "Follow the system" }).focus();
  await page.keyboard.press("Tab");
  const emeraldDark = themes.getByRole("radio", { name: "Emerald dark" });
  await expect(emeraldDark).toBeFocused();
  await expect(page.locator(".theme-card:focus-within")).toHaveCSS(
    "outline-style",
    "solid",
  );

  await page.keyboard.press("ArrowRight");
  const oled = themes.getByRole("radio", { name: "OLED black" });
  await expect(oled).toBeFocused();
  await expect(oled).toBeChecked();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "oled");

  // Back across the boundary between the rows, into the light one.
  await page.keyboard.press("ArrowLeft");
  await page.keyboard.press("ArrowLeft");
  const oceanLight = themes.getByRole("radio", { name: "Ocean light" });
  await expect(oceanLight).toBeChecked();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "ocean-light");

  // And the tab order still holds one stop: out and back lands on it.
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.press("Tab");
  await expect(oceanLight).toBeFocused();
});

test("follows the device between a preset and its pair, live", async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: "light" });
  await page.addInitScript(() => {
    if (!sessionStorage.getItem("seeded")) {
      localStorage.setItem("waveflow.theme", "midnight");
      sessionStorage.setItem("seeded", "1");
    }
  });
  await page.goto("/settings");
  const themes = picker(page);
  const html = page.locator("html");
  await expect(html).toHaveAttribute("data-theme", "midnight");

  const follow = page.getByRole("checkbox", { name: "Follow the system" });
  await expect(follow).not.toBeChecked();
  await follow.check();
  // The device is light: Midnight's light half, and its card is checked.
  await expect(html).toHaveAttribute("data-theme", "midnight-light");
  await expect(
    themes.getByRole("radio", { name: "Midnight light" }),
  ).toBeChecked();
  await expect(page.getByText("Your device is in light mode")).toBeVisible();

  // The device turns dark, and the page goes with it without a reload.
  await page.emulateMedia({ colorScheme: "dark" });
  await expect(html).toHaveAttribute("data-theme", "midnight");

  // Picking a card picks its family: Sunset light, at night, is Sunset.
  await themes.getByRole("radio", { name: "Sunset light" }).click();
  await expect(html).toHaveAttribute("data-theme", "sunset");
  await expect(themes.getByRole("radio", { name: "Sunset", exact: true })).toBeChecked();

  // Kept per browser, beside the theme, and read back on the next visit.
  await expect
    .poll(() =>
      page.evaluate(() => localStorage.getItem("waveflow.themeFollowSystem")),
    )
    .toBe("true");
  await page.emulateMedia({ colorScheme: "light" });
  await page.reload();
  await expect(html).toHaveAttribute("data-theme", "sunset-light");
  await expect(follow).toBeChecked();

  // A one-off has no light half: Emerald stands in, and the page says so.
  await themes.getByRole("radio", { name: "OLED black" }).click();
  await expect(html).toHaveAttribute("data-theme", "default");
  await expect(
    page.getByText("OLED black has no light version, so Emerald light stands in"),
  ).toBeVisible();
  await page.emulateMedia({ colorScheme: "dark" });
  await expect(html).toHaveAttribute("data-theme", "oled");

  // Turning it off keeps what is on screen rather than jumping.
  await page.emulateMedia({ colorScheme: "light" });
  await expect(html).toHaveAttribute("data-theme", "default");
  await follow.uncheck();
  await expect(html).toHaveAttribute("data-theme", "default");
  await page.emulateMedia({ colorScheme: "dark" });
  await expect(html).toHaveAttribute("data-theme", "default");
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("waveflow.theme")))
    .toBe("default");
});

test("reflows the cards at any width without scrolling sideways", async ({
  page,
}) => {
  await page.goto("/settings");
  await expect(picker(page)).toBeVisible();
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  expect(overflow).toBeLessThanOrEqual(0);
  // Every card inside the viewport's width, and at least two abreast.
  const boxes = await page.locator(".theme-card").evaluateAll((cards) =>
    cards.map((card) => card.getBoundingClientRect().toJSON()),
  );
  const width = await page.evaluate(() => document.documentElement.clientWidth);
  for (const box of boxes) expect(box.right).toBeLessThanOrEqual(width);
  const firstRow = boxes.filter((box) => box.top === boxes[0]?.top);
  expect(firstRow.length).toBeGreaterThanOrEqual(2);
});

test("drops the cards' motion when the device asks for less", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/settings");
  const card = page.locator(".theme-card").first();
  await expect(card).toBeVisible();
  const durations = await card.evaluate((element) =>
    getComputedStyle(element).transitionDuration.split(",").map((value) => value.trim()),
  );
  expect(durations.every((value) => value === "0s")).toBe(true);
});

/**
 * The screens each preset is audited on: the albums grid (the landing page),
 * Settings (the picker itself, a form, the toggles), and an album's song
 * table with a starred, rated and playing row. The play bar is on all three.
 */
const SCREENS = [
  {
    path: "/",
    ready: (page: Page) =>
      expect(page.getByRole("heading", { name: "Albums" })).toBeVisible(),
  },
  {
    path: "/settings",
    ready: (page: Page) => expect(picker(page)).toBeVisible(),
  },
  {
    path: "/albums/album-2",
    ready: (page: Page) =>
      expect(page.getByRole("row", { name: /Cocoon/ })).toBeVisible(),
  },
];

for (const preset of THEME_PRESETS) {
  test(`holds colour contrast in ${preset.id}`, async ({ page }) => {
    await page.addInitScript((id) => {
      localStorage.setItem("waveflow.theme", id);
    }, preset.id);
    for (const screen of SCREENS) {
      await page.goto(screen.path);
      await expect(page.locator("html")).toHaveAttribute("data-theme", preset.id);
      await screen.ready(page);
      await expect(page.locator(".player")).toBeVisible();
      await settle(page);
      const results = await new AxeBuilder({ page })
        .withRules(["color-contrast"])
        .analyze();
      expect(
        results.violations.flatMap((violation) =>
          violation.nodes.map((node) => `${screen.path} ${node.target.join(" ")}: ${node.failureSummary}`),
        ),
      ).toEqual([]);
    }
  });
}
