import AxeBuilder from "@axe-core/playwright";
import { expect, type Page, test } from "@playwright/test";

/**
 * The workspace pages: Radio, Podcasts, Folders and the Maintenance hub, which
 * manage something rather than present it.
 *
 * The radio mock keeps what it is sent, as the one in `studio-nocturne` does:
 * a delete that deleted nothing, or an edit that saved nothing, would
 * otherwise pass against a fixed answer.
 */

const session = {
  access_token: "e2e-access-token",
  user: { id: "user-1", username: "listener", role: "admin" },
  device_id: "device-1",
};

const LIBRARY_ID = "2740f393-e84e-488a-8a67-745deb23399a";

type Station = {
  id: string;
  name: string;
  stream_url: string;
  homepage_url: string | null;
  image_url: string | null;
  artwork_hash: string | null;
  created_at: number;
  updated_at: number;
};

let stations: Station[] = [];
/** Every radio write, by method and station, so one click can be told from two. */
let radioWrites: string[] = [];
/** Held open by the test that needs a save in flight; instant otherwise. */
let radioGate: Promise<void> = Promise.resolve();
let canManage = true;
let folders: Array<{
  id: string;
  parent_id: string | null;
  name: string;
  track_count: number;
}> = [];

async function expectNoWcagViolations(page: Page) {
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
  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  expect(results.violations).toEqual([]);
}

function stationFrom(id: string, draft: Record<string, unknown>): Station {
  return {
    id,
    name: String(draft.name).trim(),
    stream_url: String(draft.stream_url),
    homepage_url: (draft.homepage_url as string | null) ?? null,
    image_url: (draft.image_url as string | null) ?? null,
    artwork_hash: null,
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
  };
}

async function mockApi(page: Page) {
  await page.context().addCookies([
    { name: "waveflow-csrf", value: "e2e-csrf", url: "http://127.0.0.1:4173" },
  ]);
  await page.route("**/api/v2/**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const method = request.method();
    const path = url.pathname;
    if (path === "/api/v2/web/auth/refresh") {
      await route.fulfill({ json: session });
      return;
    }
    if (path === "/api/v2/playlists" || path === "/api/v2/albums") {
      await route.fulfill({ json: [] });
      return;
    }
    if (path === "/api/v2/libraries") {
      await route.fulfill({
        json: [
          {
            id: LIBRARY_ID,
            name: "Ma musique",
            visibility: "private",
            role: "owner",
            last_scan_started_at: null,
            last_scan_completed_at: null,
            accepts_uploads: false,
            accepts_canvas: false,
            follow_links: false,
          },
        ],
      });
      return;
    }
    if (path === "/api/v2/radio-stations") {
      if (method === "POST") {
        radioWrites.push("POST");
        await radioGate;
        const created = stationFrom(
          `station-${stations.length + 1}`,
          request.postDataJSON(),
        );
        stations = [...stations, created];
        await route.fulfill({ status: 201, json: created });
        return;
      }
      await route.fulfill({ json: stations });
      return;
    }
    if (path.startsWith("/api/v2/radio-stations/")) {
      const id = path.slice("/api/v2/radio-stations/".length);
      radioWrites.push(`${method} ${id}`);
      if (method === "DELETE") {
        stations = stations.filter((station) => station.id !== id);
        await route.fulfill({ status: 204, body: "" });
        return;
      }
      const replaced = stationFrom(id, request.postDataJSON());
      stations = stations.map((station) =>
        station.id === id ? replaced : station,
      );
      await route.fulfill({ json: replaced });
      return;
    }
    if (path === "/api/v2/podcasts") {
      await route.fulfill({
        json: { enabled: true, can_manage: canManage, channels: [] },
      });
      return;
    }
    if (path === `/api/v2/libraries/${LIBRARY_ID}/folders`) {
      await route.fulfill({ json: { folders, unfiled_track_count: 12 } });
      return;
    }
    if (path === `/api/v2/libraries/${LIBRARY_ID}/quality`) {
      await route.fulfill({
        json: {
          enabled: true,
          lossless: 0,
          measured: 0,
          verdicts: [],
          analysis: {
            state: "idle",
            in_flight: 0,
            pass_started_at: null,
            last_measured_at: null,
          },
        },
      });
      return;
    }
    if (path.startsWith(`/api/v2/libraries/${LIBRARY_ID}/quality/tracks`)) {
      await route.fulfill({ json: [] });
      return;
    }
    await route.fulfill({ status: 404, json: { error: "not found" } });
  });
}

test.beforeEach(async ({ page }) => {
  stations = [];
  radioWrites = [];
  radioGate = Promise.resolve();
  canManage = true;
  folders = [];
  await mockApi(page);
});

/**
 * A station is added in a dialog, listed as a card, and edited or deleted
 * from the card's "⋯" menu — a delete only once it is confirmed.
 */
test("adds a station in a dialog, and edits and deletes it from its card's menu", async ({
  page,
}) => {
  await page.goto("/radio");
  await expect(page.getByRole("heading", { name: "Radio", exact: true })).toBeVisible();
  await expect(page.getByText("No stations yet. Add one to keep it here.")).toBeVisible();

  const opener = page.getByRole("button", { name: "Add station" });
  await opener.click();
  const dialog = page.getByRole("dialog", { name: "Add a station" });
  await expect(dialog).toBeVisible();
  // The form's rules came with it into the dialog.
  await dialog.getByLabel("Station name").fill("Radio Nova");
  await dialog.getByLabel("Stream address").fill("mms://nova.example.org/live");
  await expect(
    dialog.getByText("An address must start with http:// or https://."),
  ).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Save station" })).toBeDisabled();
  await dialog.getByLabel("Stream address").fill("https://nova.example.org/live");
  await dialog.getByLabel("Station page (optional)").fill("https://nova.example.org/");
  await expect(dialog.getByLabel("Station image (optional)")).toBeVisible();
  await expectNoWcagViolations(page);
  await dialog.getByRole("button", { name: "Save station" }).click();

  await expect(dialog).toHaveCount(0);
  expect(radioWrites).toEqual(["POST"]);
  // Focus comes back to what opened the dialog.
  await expect(opener).toBeFocused();
  const grid = page.getByRole("list", { name: "Stations" });
  const card = grid.getByRole("listitem").filter({ hasText: "Radio Nova" });
  await expect(card).toHaveCount(1);
  await expect(card.getByRole("button", { name: "Listen to Radio Nova" })).toBeVisible();
  await expect(card.getByRole("link", { name: "Station page" })).toHaveAttribute(
    "href",
    "https://nova.example.org/",
  );
  // No delete button sits on the card any more: it is in the menu.
  await expect(card.getByRole("button", { name: "Delete" })).toHaveCount(0);
  await expectNoWcagViolations(page);

  // The menu, by keyboard: it opens on its first entry, the arrows move
  // through it, and Escape gives focus back to the button.
  const more = card.getByRole("button", { name: "Actions for Radio Nova" });
  await more.click();
  const menu = page.getByRole("menu", { name: "Actions for Radio Nova" });
  await expect(menu.getByRole("menuitem")).toHaveText(["Edit", "Delete"]);
  await expect(menu.getByRole("menuitem", { name: "Edit" })).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(menu.getByRole("menuitem", { name: "Delete" })).toBeFocused();
  await expectNoWcagViolations(page);
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);
  await expect(more).toBeFocused();

  // Editing opens the same form, filled, under the station's name.
  await more.click();
  await page.getByRole("menuitem", { name: "Edit" }).click();
  const editor = page.getByRole("dialog", { name: "Edit “Radio Nova”" });
  await expect(editor.getByLabel("Station name")).toHaveValue("Radio Nova");
  await expect(
    editor.getByText("Saving replaces every field", { exact: false }),
  ).toBeVisible();
  await editor.getByLabel("Station name").fill("Radio Nova Classics");
  await editor.getByRole("button", { name: "Save station" }).click();
  await expect(editor).toHaveCount(0);
  await expect(grid.getByRole("listitem")).toHaveText([/Radio Nova Classics/]);
  expect(radioWrites).toEqual(["POST", "PUT station-1"]);

  // A delete is asked first; declined, nothing is sent.
  const renamed = grid.getByRole("listitem").filter({ hasText: "Radio Nova Classics" });
  page.once("dialog", (question) => {
    expect(question.message()).toContain("Radio Nova Classics");
    void question.dismiss();
  });
  await renamed.getByRole("button", { name: "Actions for Radio Nova Classics" }).click();
  await page.getByRole("menuitem", { name: "Delete" }).click();
  await expect(grid.getByRole("listitem")).toHaveCount(1);
  expect(radioWrites).toEqual(["POST", "PUT station-1"]);

  // Confirmed, it goes.
  page.once("dialog", (question) => void question.accept());
  await renamed.getByRole("button", { name: "Actions for Radio Nova Classics" }).click();
  await page.getByRole("menuitem", { name: "Delete" }).click();
  await expect(page.getByText("No stations yet. Add one to keep it here.")).toBeVisible();
  expect(radioWrites).toEqual(["POST", "PUT station-1", "DELETE station-1"]);
});

/**
 * The dialog cannot be put away while it saves: Escape would otherwise close
 * the form under a request that is about to report into it.
 */
test("holds the station dialog open while its save is in flight", async ({ page }) => {
  let release = () => {};
  radioGate = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.goto("/radio");
  await page.getByRole("button", { name: "Add station" }).click();
  const dialog = page.getByRole("dialog", { name: "Add a station" });
  await dialog.getByLabel("Station name").fill("FIP");
  await dialog.getByLabel("Stream address").fill("https://fip.example.org/live");
  await dialog.getByRole("button", { name: "Save station" }).click();
  await expect.poll(() => radioWrites).toEqual(["POST"]);

  await expect(dialog.getByLabel("Station name")).toBeDisabled();
  await expect(dialog.getByRole("button", { name: "Close" })).toBeDisabled();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeVisible();

  release();
  await expect(dialog).toHaveCount(0);
  await expect(
    page.getByRole("list", { name: "Stations" }).getByRole("listitem"),
  ).toHaveText([/FIP/]);
});

/**
 * An empty podcasts page says what podcasts do on this server, to everyone;
 * a manager is also given the first step, which is the feed field.
 */
test("explains an empty podcasts page, and offers a manager the first step", async ({
  page,
}) => {
  await page.goto("/podcasts");
  const empty = page.locator(".empty-state");
  await expect(empty).toContainText("No podcasts yet.");
  await expect(empty).toContainText(
    "An administrator subscribes to a show’s feed here. The server downloads its episodes, and every account on this server can listen to them.",
  );
  await expect(empty.locator("svg")).toBeVisible();
  await expectNoWcagViolations(page);
  await empty.getByRole("button", { name: "Add your first podcast" }).click();
  await expect(page.getByLabel("Feed address")).toBeFocused();
});

test("explains an empty podcasts page to a reader, with nothing to press", async ({
  page,
}) => {
  canManage = false;
  await page.goto("/podcasts");
  const empty = page.locator(".empty-state");
  await expect(empty).toContainText("every account on this server can listen to them");
  await expect(empty.getByRole("button")).toHaveCount(0);
  await expect(page.getByLabel("Feed address")).toHaveCount(0);
  await expectNoWcagViolations(page);
});

/**
 * "To sort" is WaveFlow's, not a folder anyone made, and the header counts
 * only the folders that were made: so it is drawn apart from them, under a
 * heading of its own that says what it is.
 */
test("draws To sort apart from the folders the header counts", async ({ page }) => {
  folders = [{ id: "folder-1", parent_id: null, name: "Jazz", track_count: 4 }];
  await page.goto("/folders");
  await expect(page.getByText("1 folder created in this library")).toBeVisible();
  const made = page.getByRole("list", { name: "Folders" });
  await expect(made.getByRole("link")).toHaveCount(1);
  await expect(made.getByRole("link", { name: /Jazz/ })).toBeVisible();
  await expect(made.getByRole("link", { name: /To sort/ })).toHaveCount(0);

  const kept = page.getByRole("region", { name: "Kept by WaveFlow" });
  await expect(kept).toContainText(
    "it is not counted among the folders above, and it cannot be renamed, moved or deleted",
  );
  const tray = kept.getByRole("list", { name: "Kept by WaveFlow" }).getByRole("link", {
    name: /To sort/,
  });
  await expect(tray).toContainText("12 tracks");
  await expect(tray).toContainText("Automatic");
  await expectNoWcagViolations(page);

  await tray.click();
  await expect(page.getByRole("heading", { name: "To sort" })).toBeVisible();
});

test("says no folder was made yet, and still offers To sort", async ({ page }) => {
  await page.goto("/folders");
  await expect(page.getByText("0 folders created in this library")).toBeVisible();
  await expect(
    page.getByText("No folder has been created in this library yet."),
  ).toBeVisible();
  await expect(page.getByRole("list", { name: "Folders" })).toHaveCount(0);
  await expect(
    page.getByRole("list", { name: "Kept by WaveFlow" }).getByRole("link", { name: /To sort/ }),
  ).toBeVisible();
});

const WORKSPACE_PAGES = ["/radio", "/podcasts", "/folders", "/maintenance/quality"];

async function mainWidth(page: Page, path: string) {
  await page.goto(path);
  await expect(page.locator("main h2").first()).toBeVisible();
  return page.locator("main").evaluate((main) => main.getBoundingClientRect().width);
}

/**
 * On a wide window the workspace pages take more of it, and the editorial
 * pages keep the contained column they were designed in.
 */
test("gives the workspace pages more of a wide window, and not the editorial ones", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "a wide window");
  await page.setViewportSize({ width: 2560, height: 1440 });
  stations = [stationFrom("station-1", { name: "Radio Nova", stream_url: "https://nova.example.org/live" })];
  const editorial = await mainWidth(page, "/");
  // 105rem, the contained column.
  expect(editorial).toBe(1680);
  for (const path of WORKSPACE_PAGES) {
    expect(await mainWidth(page, path), path).toBeGreaterThan(editorial + 400);
  }
  await expectNoWcagViolations(page);
});

/**
 * And on a phone, or beside the compact rail of a short window, they still
 * fit: the 16 px gutter, and nothing to scroll sideways.
 */
test("keeps the workspace pages inside a phone and a short window", async ({
  page,
}, testInfo) => {
  if (testInfo.project.name === "desktop") {
    await page.setViewportSize({ width: 1024, height: 640 });
  }
  stations = [
    stationFrom("station-1", {
      name: "A station whose name is far too long to fit on one card",
      stream_url: "https://a-very-long-host-name.example.org/a/very/long/stream/path.mp3",
      homepage_url: "https://example.org/",
    }),
    stationFrom("station-2", { name: "FIP", stream_url: "https://fip.example.org/live" }),
  ];
  folders = [{ id: "folder-1", parent_id: null, name: "Jazz", track_count: 4 }];
  for (const path of WORKSPACE_PAGES) {
    await page.goto(path);
    await expect(page.locator("main h2").first()).toBeVisible();
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    );
    expect(overflow, path).toBe(0);
    if (testInfo.project.name === "mobile") {
      const gutter = await page
        .locator("main")
        .evaluate((main) => getComputedStyle(main).paddingLeft);
      expect(gutter, path).toBe("16px");
    }
  }
  await page.goto("/radio");
  await expect(page.getByRole("list", { name: "Stations" }).getByRole("listitem")).toHaveCount(2);
  await expectNoWcagViolations(page);
});
