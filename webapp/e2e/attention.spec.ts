// Needs attention: the credits waiting for a decision (RFC-015).

import { expect, test } from "@playwright/test";

import {
  chooseLightTheme,
  expectNoWcagViolations,
} from "./fixtures/accessibility";
import { resetAndMockApi } from "./fixtures/api";
import { serveAttention } from "./fixtures/attention";
import { artistDetail, library } from "./fixtures/catalogue";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

test("lists what needs attention, splits a credit on the second click, and undoes it", async ({
  page,
}) => {
  const state = await serveAttention(page);
  await page.goto("/attention");
  await expect(
    page.getByRole("heading", { name: "Needs attention" }),
  ).toBeVisible();
  await expect(page.getByText("2 things to look at")).toBeVisible();
  // The first list holding something is open, and an empty one stays drawn.
  const lists = page.getByRole("list", { name: "Lists" });
  await expect(
    lists.getByRole("link", { name: /Artists to split/ }),
  ).toHaveAttribute("aria-current", "page");
  await expect(lists.getByRole("link", { name: /Missing cover/ })).toHaveClass(
    /empty/,
  );

  // A fragment the library already knows leads to that artist.
  const fragments = page.getByRole("list", {
    name: "What Ashreveal, Ashish Swargiary would split into",
  });
  await expect(
    fragments.getByRole("link", { name: "Ashreveal" }),
  ).toHaveAttribute("href", "/artists/artist-known");
  await expect(fragments.getByText("Ashish Swargiary")).toBeVisible();

  // The first click arms, and sends nothing.
  await page.getByRole("button", { name: "Split", exact: true }).click();
  const confirm = page.getByRole("button", { name: "Confirm: split into 2" });
  await expect(confirm).toBeVisible();
  expect(state.writes).toEqual([]);
  await expectNoWcagViolations(page);
  // The confirmation stands for four seconds, and on a loaded machine the scan
  // alone can outlast them: armed again if it lapsed, so the click below meets
  // the confirmation rather than racing its timer.
  if (!(await confirm.isVisible())) {
    await page.getByRole("button", { name: "Split", exact: true }).click();
  }
  await confirm.click();
  await expect(
    page.getByText("Ashreveal, Ashish Swargiary is split."),
  ).toBeVisible();
  expect(state.writes).toEqual([
    {
      method: "POST",
      path: "credit-decisions",
      body: { artist_id: "artist-joined", decision: "split" },
    },
  ]);
  await expect(
    page.getByText("No credit is waiting for a decision."),
  ).toBeVisible();

  // Decided, then undone.
  await page.getByRole("button", { name: "Decided" }).click();
  await expect(
    page.getByText("Split into Ashreveal · Ashish Swargiary"),
  ).toBeVisible();
  await page.getByRole("button", { name: "Undo" }).click();
  await expect(
    page.getByText("The decision about Ashreveal, Ashish Swargiary is undone."),
  ).toBeVisible();
  expect(state.writes.at(-1)).toEqual({
    method: "DELETE",
    path: "decision-1",
    body: null,
  });

  // A track list opens its tracks in the song table.
  await lists.getByRole("link", { name: /Missing year/ }).click();
  await expect(page).toHaveURL(/category=missing_year/);
  await expect(page.getByText("Yearless")).toBeVisible();

  // Probable duplicates are drawn group by group.
  await lists.getByRole("link", { name: /Probable duplicates/ }).click();
  await expect(
    page.getByRole("heading", { name: "Probable duplicate 1" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Probable duplicate 2" }),
  ).toBeVisible();
  await expect(page.locator(".attention-group")).toHaveCount(2);
  await expect(
    page.locator(".attention-group").first().getByText("Echo (copy)"),
  ).toBeVisible();
  await expectNoWcagViolations(page);
});

/**
 * Two owners deciding the same credit: the second is refused, and the list is
 * read again rather than left showing a row that is no longer there.
 */
test("reloads the credits when a decision is refused", async ({ page }) => {
  const state = await serveAttention(page);
  state.refuseWith = 409;
  await page.goto("/attention?category=split_credits");
  await page.getByRole("button", { name: "Keep as one" }).click();
  await expect(
    page.getByText(
      "This credit was decided in the meantime. The list has been reloaded.",
    ),
  ).toBeVisible();
  await expect(
    page.getByText("No credit is waiting for a decision."),
  ).toBeVisible();
});

/**
 * A listener cannot act on any of it, so the page says so and asks nothing:
 * the server would answer `404` regardless.
 */
test("asks nothing of the server for a listener's library", async ({
  page,
}) => {
  mock.libraries = [
    { ...library("library-1", "Ma musique"), role: "listener" },
  ];
  const state = await serveAttention(page);
  await page.goto("/attention");
  await expect(
    page.getByText(
      "Only the owners and managers of this library see what needs attention in it.",
    ),
  ).toBeVisible();
  // Settled before counting: the message and a request fired beside it
  // arrive together, and a count taken first would be zero either way.
  await page.waitForLoadState("networkidle");
  expect(state.asks).toBe(0);
});

/** The desktop's other way in: from the joined artist's own page. */
test("splits a joined artist from its page, after a confirming click", async ({
  page,
}) => {
  const state = await serveAttention(page);
  await page.route("**/api/v2/artists/artist-joined", (route) =>
    route.fulfill({
      json: {
        ...artistDetail,
        id: "artist-joined",
        name: "Ashreveal, Ashish Swargiary",
        library_id: "library-1",
        album_count: 0,
        albums: [],
      },
    }),
  );
  await page.route("**/api/v2/artists/artist-joined/*", (route) =>
    route.fulfill({ json: [] }),
  );
  await page.goto("/artists/artist-joined");
  const offer = page.getByRole("button", { name: "Split into 2 artists" });
  await expect(offer).toBeVisible();
  await offer.click();
  await page.getByRole("button", { name: "Confirm: split into 2" }).click();
  await expect(page).toHaveURL(/\/artists$/);
  expect(state.writes).toEqual([
    {
      method: "POST",
      path: "credit-decisions",
      body: { artist_id: "artist-joined", decision: "split" },
    },
  ]);

  // A listener is offered nothing.
  mock.libraries = [
    { ...library("library-1", "Ma musique"), role: "listener" },
  ];
  await page.goto("/artists/artist-joined");
  await expect(
    page.getByRole("heading", { name: "Ashreveal, Ashish Swargiary" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Split into 2 artists" }),
  ).toHaveCount(0);
});

test("keeps Needs attention readable in the light", async ({ page }) => {
  await serveAttention(page);
  await chooseLightTheme(page);
  await page.goto("/attention");
  await expect(
    page.getByText("Ashreveal, Ashish Swargiary").first(),
  ).toBeVisible();
  await expectNoWcagViolations(page);
});

/**
 * The light presets share one material and differ by their ground's tint and
 * their accent's 700 stop, which is also the colour of every link and every
 * open item: each one is audited where the accent is spent most.
 */
for (const theme of [
  "default",
  "midnight-light",
  "sunset-light",
  "lavender-light",
  "crimson-light",
  "ocean-light",
]) {
  test(`keeps the ${theme} theme readable`, async ({ page }) => {
    await serveAttention(page);
    await page.addInitScript((id) => {
      localStorage.setItem("waveflow.theme", id);
    }, theme);
    await page.goto("/attention");
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await expect(
      page.getByText("Ashreveal, Ashish Swargiary").first(),
    ).toBeVisible();
    await expectNoWcagViolations(page);
  });
}
