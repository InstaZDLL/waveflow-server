// The account's transcoding profile, in Settings > Playback (RFC-027).

import { expect, type Page, test } from "@playwright/test";

import {
  chooseLightTheme,
  expectNoWcagViolations,
} from "./fixtures/accessibility";
import { resetAndMockApi } from "./fixtures/api";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

const allApps = (page: Page) => page.getByRole("form", { name: "All apps" });
const oneApp = (page: Page) => page.getByRole("form", { name: "For one app" });

/**
 * The default is written to the server and read back, a bit rate left empty
 * comes back as the number the format defaults to, and Original removes the row
 * rather than storing one: the mock keeps the rows, so a save that saved nothing
 * would not pass.
 */
test("sets the account's default and takes it away again", async ({ page }) => {
  await page.goto("/settings/playback");
  const form = allApps(page);
  await expect(form.getByLabel("Format")).toHaveValue("raw");
  await expect(form.getByLabel(/bit rate/)).toHaveCount(0);

  await form.getByLabel("Format").selectOption("opus");
  await form.getByRole("button", { name: "Save" }).click();
  await expect(form.getByText("Saved.")).toBeVisible();
  await expect(form.getByLabel(/bit rate/)).toHaveValue("128");
  expect(mock.transcodingProfiles.profiles).toMatchObject([
    { client: null, format: "opus", bitrate: 128 },
  ]);

  await form.getByLabel(/bit rate/).fill("96");
  await form.getByRole("button", { name: "Save" }).click();
  await expect
    .poll(() => mock.transcodingProfiles.profiles[0]?.bitrate)
    .toBe(96);

  await page.reload();
  await expect(allApps(page).getByLabel("Format")).toHaveValue("opus");
  await expect(allApps(page).getByLabel(/bit rate/)).toHaveValue("96");

  await allApps(page).getByLabel("Format").selectOption("raw");
  await allApps(page).getByRole("button", { name: "Save" }).click();
  await expect.poll(() => mock.transcodingProfiles.profiles).toEqual([]);
});

/**
 * The formats offered are the ones the server says it can produce, and a bit
 * rate outside the server's bounds is refused here, in the page's own language,
 * without a request.
 */
test("offers the server's formats and says when a bit rate is out of bounds", async ({
  page,
}) => {
  mock.transcodeStatus = { ...mock.transcodeStatus, formats: ["mp3", "aac"] };
  await page.goto("/settings/playback");
  const form = allApps(page);
  await expect(form.getByLabel("Format").locator("option")).toHaveText([
    "Original",
    "MP3",
    "AAC",
  ]);

  await form.getByLabel("Format").selectOption("mp3");
  await form.getByLabel(/bit rate/).fill("600");
  await form.getByRole("button", { name: "Save" }).click();
  await expect(form.getByRole("alert")).toHaveText(
    "The bit rate has to be between 32 and 512 kbit/s.",
  );
  expect(mock.transcodingProfiles.profiles).toEqual([]);
});

/**
 * A row whose format the server no longer lists is shown as what it is, and can
 * still be taken away: with no option for it, the select would show Original
 * while the page held Opus, and choosing Original would change nothing.
 */
test("shows a stored format the server no longer lists, and removes it", async ({
  page,
}) => {
  mock.transcodeStatus = { ...mock.transcodeStatus, formats: ["mp3"] };
  mock.transcodingProfiles.profiles = [
    {
      client: null,
      format: "opus",
      bitrate: 128,
      updated_at: 1_700_000_000_000,
    },
  ];
  await page.goto("/settings/playback");
  const form = allApps(page);
  await expect(form.getByLabel("Format")).toHaveValue("opus");
  await expect(form.getByLabel("Format").locator("option")).toHaveText([
    "Original",
    "MP3",
    "Opus",
  ]);

  await form.getByLabel("Format").selectOption("raw");
  await form.getByRole("button", { name: "Save" }).click();
  await expect.poll(() => mock.transcodingProfiles.profiles).toEqual([]);
});

/**
 * An app's own row, including the remedy the panel gives for an app that cannot
 * seek in a transcode: a row of Original. A name seen on a silent stream is
 * suggested until it has a row.
 */
test("gives one app its own setting, Original included", async ({ page }) => {
  mock.transcodingProfiles.seen_clients = ["Symfonium", "Feishin"];
  await page.goto("/settings/playback");
  await expect(
    page.getByText("add it below with Original", { exact: false }),
  ).toBeVisible();
  await expect(
    page.getByText("No app has a setting of its own."),
  ).toBeVisible();

  const form = oneApp(page);
  const suggested = form.locator("datalist option");
  await expect(suggested).toHaveCount(2);

  await form.getByLabel("App name").fill("Feishin");
  await form.getByRole("button", { name: "Add" }).click();
  const list = page.locator(".settings-panel .resource-list");
  await expect(list.getByText("Feishin")).toBeVisible();
  await expect(list.getByText("Original")).toBeVisible();
  await expect(form.getByLabel("App name")).toHaveValue("");
  await expect(suggested).toHaveCount(1);

  await form.getByLabel("App name").fill("Symfonium");
  await form.getByLabel("Format").selectOption("opus");
  await form.getByLabel(/bit rate/).fill("128");
  await form.getByRole("button", { name: "Add" }).click();
  await expect(list.getByText("Opus · 128 kbit/s")).toBeVisible();

  await page.getByRole("button", { name: "Remove: Feishin" }).click();
  await expect(list.getByText("Feishin")).toHaveCount(0);
  expect(mock.transcodingProfiles.profiles).toMatchObject([
    { client: "Symfonium", format: "opus", bitrate: 128 },
  ]);
});

/** A refused write says so, and what could not be loaded is not a blank form. */
test("says when a profile cannot be saved or read", async ({ page }) => {
  await page.goto("/settings/playback");
  mock.transcodingProfilesFail = true;
  await allApps(page).getByLabel("Format").selectOption("opus");
  await allApps(page).getByRole("button", { name: "Save" }).click();
  await expect(allApps(page).getByRole("alert")).toHaveText(
    "That could not be saved, so nothing changed.",
  );

  await page.reload();
  await expect(
    page.locator(".settings-panel").getByRole("alert"),
  ).toContainText("We could not load this view");
});

test("keeps the panel readable in light", async ({ page }) => {
  mock.transcodingProfiles.profiles = [
    {
      client: "Symfonium",
      format: "opus",
      bitrate: 128,
      updated_at: 1_700_000_000_000,
    },
  ];
  await chooseLightTheme(page);
  await page.goto("/settings/playback");
  await expect(page.getByText("Opus · 128 kbit/s")).toBeVisible();
  await expectNoWcagViolations(page);
});
