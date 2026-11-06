// Telling an administrator a new version is out (RFC-028): the question while
// nobody has answered, the announcement once a version is known, and the
// switch in Settings > Administration.

import { expect, type Page, test } from "@playwright/test";

import { resetAndMockApi } from "./fixtures/api";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

const status = (overrides: Record<string, unknown> = {}) => ({
  policy: "ask",
  decision: true,
  enabled: true,
  current: "2.1.2",
  channel: "stable",
  latest: {
    version: "2.2.0",
    tag: "v2.2.0",
    published_at: "2026-11-10T00:00:00Z",
    page: "https://github.com/InstaZDLL/waveflow-server/releases/tag/v2.2.0",
    notes: null,
  },
  update_available: true,
  checked_at: 1_700_000_000_000,
  failure: null,
  ...overrides,
});

const question = (page: Page) =>
  page.getByRole("dialog", { name: "Check for new versions?" });
const announcement = (page: Page) =>
  page.getByRole("dialog", { name: "WaveFlow Server 2.2.0 is available" });

/** Waits for the shell, so a dialog that would open has had its answer. */
async function shellReady(page: Page) {
  await expect(page.locator("#main-content")).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => document.readyState))
    .toBe("complete");
}

test("asks an administrator once, and records the answer", async ({ page }) => {
  mock.updateStatus = status({
    decision: null,
    enabled: false,
    latest: null,
    update_available: false,
    checked_at: null,
  });
  await page.goto("/");
  await expect(question(page)).toBeVisible();

  await question(page)
    .getByRole("button", { name: "Check once a day" })
    .click();
  await expect(question(page)).toBeHidden();
  expect(mock.updateWrites).toEqual([true]);

  // Answered: the next visit asks nothing.
  await page.reload();
  await shellReady(page);
  await expect(question(page)).toHaveCount(0);
});

test("announces a newer version and keeps it dismissed", async ({ page }) => {
  mock.updateStatus = status();
  await page.goto("/");
  await expect(announcement(page)).toBeVisible();
  await expect(
    announcement(page).getByRole("link", {
      name: "Release notes and downloads",
    }),
  ).toHaveAttribute(
    "href",
    "https://github.com/InstaZDLL/waveflow-server/releases/tag/v2.2.0",
  );

  // Later closes it for this page only.
  await announcement(page).getByRole("button", { name: "Later" }).click();
  await expect(announcement(page)).toBeHidden();
  await page.reload();
  await expect(announcement(page)).toBeVisible();

  // Dismissed for this version, it stays closed.
  await announcement(page)
    .getByRole("button", { name: "Don't show this version again" })
    .click();
  await expect(announcement(page)).toBeHidden();
  await page.reload();
  await shellReady(page);
  await expect(announcement(page)).toHaveCount(0);
});

test("says nothing when the route is refused", async ({ page }) => {
  // The mock's default: the 404 every other spec runs under.
  await page.goto("/");
  await shellReady(page);
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("turns the check off from Settings > Administration", async ({ page }) => {
  mock.updateStatus = status({ update_available: false });
  await page.goto("/settings/admin");
  const panel = page.locator(".updates-panel");
  await expect(
    panel.getByText(
      "This server checks download.waveflow.app for new versions once a day.",
    ),
  ).toBeVisible();
  await panel.getByRole("button", { name: "Stop checking" }).click();
  await expect(
    panel.getByText("This server does not check for new versions."),
  ).toBeVisible();
  expect(mock.updateWrites).toEqual([false]);
});
