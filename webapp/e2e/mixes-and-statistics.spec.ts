// The day's mixes on the home page, and this account's statistics.

import { expect, test } from "@playwright/test";

import { expectNoWcagViolations } from "./fixtures/accessibility";
import { resetAndMockApi } from "./fixtures/api";
import { dailyMixes } from "./fixtures/catalogue";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

test("offers the day's mixes above the albums, and plays one", async ({
  page,
}) => {
  mock.mixes = dailyMixes();
  // Asked within the active library, like every other catalogue read.
  const scopes: Array<string | null> = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname === "/api/v2/mixes") {
      scopes.push(url.searchParams.get("library_id"));
    }
  });
  await page.goto("/");
  const shelf = page.locator(".mix-shelf");
  await expect(
    shelf.getByRole("heading", { name: "Mixes for you" }),
  ).toBeVisible();
  // Named by the client, in the listener's language, from what the server
  // sent: a kind and its parameter, never a title.
  await expect(shelf.getByRole("link")).toHaveText([
    /Most played/,
    /Art Pop mix/,
    /The 1990s/,
  ]);
  // The albums below are what they were: the shelf takes no card from them.
  await expect(page.locator(".grid li")).toHaveCount(2);
  expect(scopes).toEqual(["library-1"]);

  await shelf.getByRole("button", { name: "Play: Most played" }).click();
  await expect(page.locator(".player-track strong")).toHaveText("Hidden Place");
  expect(mock.mixAsks).toEqual(["most-played"]);
  await expectNoWcagViolations(page);

  // A genre's name travels in the address whole, space and all.
  await shelf.getByRole("link", { name: /Art Pop mix/ }).click();
  await expect(
    page.getByRole("heading", { name: "Art Pop mix" }),
  ).toBeVisible();
  await expect(page.getByText("2 tracks, drawn for today")).toBeVisible();
  expect(mock.mixAsks).toEqual(["most-played", "genre:art pop"]);
  await expectNoWcagViolations(page);

  // Filtering the albums leaves the suggestion out of the way.
  await page.goto("/");
  await expect(shelf).toBeVisible();
  await page.getByLabel("Filter by title or artist").fill("Post");
  await expect(shelf).toHaveCount(0);
});

test("says so when a mix cannot be opened, and lets the press be tried again", async ({
  page,
}) => {
  mock.mixes = dailyMixes();
  mock.mixFails = true;
  await page.goto("/");
  const shelf = page.locator(".mix-shelf");
  await shelf.getByRole("button", { name: "Play: Most played" }).click();
  await expect(shelf.getByRole("alert")).toHaveText(
    "“Most played” could not be opened. Try again.",
  );
  // Both actions say it, and neither stays disabled after the failure.
  await shelf
    .getByRole("button", { name: "Add to queue: Art Pop mix" })
    .click();
  await expect(shelf.getByRole("alert")).toHaveText(
    "“Art Pop mix” could not be opened. Try again.",
  );
  await expectNoWcagViolations(page);

  mock.mixFails = false;
  await shelf.getByRole("button", { name: "Play: Most played" }).click();
  await expect(page.locator(".player-track strong")).toHaveText("Hidden Place");
  await expect(shelf.getByRole("alert")).toHaveCount(0);
});

test("shows no shelf when the server has nothing to suggest", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(page.locator(".grid li")).toHaveCount(2);
  await expect(page.locator(".mix-shelf")).toHaveCount(0);
});

test("says a mix that is gone is gone, rather than drawing an empty one", async ({
  page,
}) => {
  await page.goto("/mixes/never-played");
  // The page's own alert. The player bar may raise one of its own about the
  // restored queue, which the mocks cannot stream; that one is not this
  // test's subject.
  await expect(page.locator("main").getByRole("alert")).toContainText(
    "We could not load this view",
  );
});

test("shows this account's listening, period by period", async ({ page }) => {
  await page.goto("/stats");
  await expect(page.getByRole("heading", { name: "Statistics" })).toBeVisible();
  const totals = page.locator(".stats-totals");
  await expect(totals).toContainText("Listens");
  await expect(totals).toContainText("Listening time");
  // A bar per day of the default thirty, the empty ones included.
  await expect(page.locator(".stats-timeline li")).toHaveCount(30);
  await expect(
    page.getByRole("listitem", { name: "2026-10-30: 4 listens" }),
  ).toBeVisible();
  await expect(page.getByRole("heading", { name: "Top tracks" })).toBeVisible();
  const columns = page.locator(".stats-columns");
  await expect(
    columns.getByRole("link", { name: "Björk", exact: true }),
  ).toBeVisible();
  await expect(columns.getByRole("link", { name: "Art Pop" })).toBeVisible();
  await expect(columns.getByRole("link", { name: /Vespertine/ })).toBeVisible();
  await expect(page.locator(".stats-moments")).toContainText("Cocoon, ");
  // Every track, artist and album entry carries its cover (or the lettered
  // placeholder when there is none); a genre has no artwork to show.
  const entries = columns.locator(".stats-entry");
  await expect(columns.locator(".stats-entry .stats-art")).toHaveCount(
    (await entries.count()) - 1,
  );
  await expect(
    columns
      .getByRole("link", { name: "Björk", exact: true })
      .locator(".stats-art-round"),
  ).toHaveCount(1);
  await expect(
    columns.getByRole("link", { name: "Art Pop" }).locator(".stats-art"),
  ).toHaveCount(0);
  // A top track is a button, and a button holds phrasing content only: the
  // lettered placeholder inside it must not be a `div`.
  await expect(columns.locator("button .cover-fallback")).toHaveCount(2);
  await expect(columns.locator("button div")).toHaveCount(0);
  await expectNoWcagViolations(page);

  await page.getByRole("combobox", { name: "Period" }).selectOption("7d");
  await expect(page.locator(".stats-timeline li")).toHaveCount(7);
  await page.getByRole("combobox", { name: "Period" }).selectOption("365d");
  await expect(page.locator(".stats-timeline li")).toHaveCount(12);
  await expect(
    page.getByRole("heading", { name: "Listens per month" }),
  ).toBeVisible();
  expect(mock.statsAsks).toEqual(["30d", "7d", "365d"]);

  // A top track plays from the list, with the others after it.
  await columns.getByRole("button", { name: /Cocoon/ }).click();
  await expect(page.locator(".player-track strong")).toHaveText("Cocoon");
});

test("keeps the statistics and the shelf readable in the light", async ({
  page,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem("waveflow.theme", "default");
  });
  mock.mixes = dailyMixes();
  await page.goto("/");
  await expect(page.locator(".mix-shelf")).toBeVisible();
  await expectNoWcagViolations(page);
  await page.goto("/stats");
  await expect(page.locator(".stats-timeline li")).toHaveCount(30);
  await expectNoWcagViolations(page);
});
