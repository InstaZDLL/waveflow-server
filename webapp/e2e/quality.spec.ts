// The quality page: what each file's audio says (RFC-016).

import { expect, test } from "@playwright/test";

import {
  chooseLightTheme,
  expectNoWcagViolations,
} from "./fixtures/accessibility";
import { resetAndMockApi } from "./fixtures/api";
import { library } from "./fixtures/catalogue";
import { analysisAt, serveQuality } from "./fixtures/quality";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

test("lists a library's files by what their audio says, worst first", async ({
  page,
}) => {
  const state = await serveQuality(page);
  await page.goto("/quality");
  await expect(page.getByRole("heading", { name: "Quality" })).toBeVisible();
  await expect(page.getByText("2 of 3 lossless files analysed")).toBeVisible();

  const verdicts = page.getByRole("list", { name: "Verdicts" });
  await expect(
    verdicts.getByRole("link", { name: /Probably from a lossy file/ }),
  ).toContainText("1");
  const rows = page.locator(".quality-table tbody tr");
  await expect(rows).toHaveCount(2);
  await expect(rows.first()).toContainText("Made From An MP3");
  // The cover before the title: a name alone is hard to place.
  await expect(rows.first().locator(".song-cover")).toHaveCount(1);
  await expect(rows.first()).toContainText("16.4 kHz");
  await expect(rows.first()).toContainText("Probably from a lossy file");
  await expect(rows.nth(1)).toContainText("Full band");
  await expect(rows.nth(1)).toContainText("No sign of a lossy source");
  await expectNoWcagViolations(page);

  // A verdict filters through the server, and is written in the address.
  await verdicts
    .getByRole("link", { name: /Probably from a lossy file/ })
    .click();
  await expect(page).toHaveURL(/verdict=lossy_source/);
  await expect(rows).toHaveCount(1);
  expect(
    state.asked.some((search) => search.includes("verdict=lossy_source")),
  ).toBe(true);

  // A file opens on its real quality: the verdict, the numbers behind it, the
  // spectrum it was judged on, and no spectrogram until somebody asks.
  await page
    .getByRole("button", { name: /^Made From An MP3/ })
    .click();
  const dialog = page.getByRole("dialog");
  await expect(
    dialog.getByRole("heading", { name: "Real quality" }),
  ).toBeVisible();
  await expect(
    dialog.getByRole("img", {
      name: "Averaged spectrum: the highs stop at 16.4 kHz.",
    }),
  ).toBeVisible();
  expect(state.spectrograms).toBe(0);
  await dialog.getByRole("button", { name: "Show the spectrogram" }).click();
  await expect(
    dialog.getByRole("img", { name: /Spectrogram of Made From An MP3/ }),
  ).toBeVisible();
  expect(state.spectrograms).toBe(1);
  await expectNoWcagViolations(page);
});

test("asks for the whole library to be analysed again on the second click", async ({
  page,
}) => {
  const state = await serveQuality(page);
  await page.goto("/quality");
  await page.getByRole("button", { name: "Analyse again" }).click();
  expect(state.writes).toEqual([]);
  await page
    .getByRole("button", { name: "Confirm: analyse the whole library again" })
    .click();
  await expect(
    page.getByText("The library will be analysed again in the background."),
  ).toBeVisible();
  expect(state.writes).toEqual(["POST"]);
});

test("shows a listener the quality, and nothing to ask for", async ({
  page,
}) => {
  mock.libraries = [{ ...library("library-1", "Ma musique"), role: "listener" }];
  await serveQuality(page);
  await page.goto("/quality");
  await expect(page.locator(".quality-table tbody tr")).toHaveCount(2);
  await expect(
    page.getByRole("button", { name: "Analyse again" }),
  ).toHaveCount(0);
});

test("keeps the quality page readable in the light", async ({ page }) => {
  await serveQuality(page);
  await chooseLightTheme(page);
  await page.goto("/quality");
  await expect(page.locator(".quality-table tbody tr")).toHaveCount(2);
  await expectNoWcagViolations(page);
});

test("shows the analysis moving, then at rest, and stops asking", async ({
  page,
}) => {
  const state = await serveQuality(page);
  const running = (measured: number) => ({
    measured,
    analysis: {
      state: "running",
      in_flight: 1,
      pass_started_at: 1_700_000_000_000,
      last_measured_at: 1_700_000_000_000,
    },
  });
  state.analyses = [
    running(1),
    running(2),
    {
      measured: 3,
      analysis: {
        state: "idle",
        in_flight: 0,
        pass_started_at: null,
        last_measured_at: 1_700_000_000_000,
      },
    },
  ];
  await page.goto("/quality");
  const status = page.getByRole("status").filter({ hasText: /Analysing|Idle/ });
  await expect(status).toHaveText(
    "Analysing now: 1 of 3 done, 1 in progress.",
  );
  // The count moves under the reader while the pass runs.
  await expect(status).toHaveText(
    "Analysing now: 2 of 3 done, 1 in progress.",
    { timeout: 10_000 },
  );
  await expect(status).toHaveText(/^Idle\. Last file analysed /, {
    timeout: 10_000,
  });
  // At rest, nothing is asked again.
  const asked = state.summaries;
  await page.waitForTimeout(4_000);
  expect(state.summaries).toBe(asked);
  await expectNoWcagViolations(page);
});

test("says the analysis waits for the scan", async ({ page }) => {
  const state = await serveQuality(page);
  state.analyses = [
    {
      measured: 0,
      analysis: {
        state: "waiting_for_scan",
        in_flight: 0,
        pass_started_at: null,
        last_measured_at: null,
      },
    },
  ];
  await page.goto("/quality");
  await expect(
    page.getByText("Waiting for the scan to finish before analysing."),
  ).toBeVisible();
});

test("offers no new analysis while one runs", async ({ page }) => {
  const state = await serveQuality(page);
  state.analyses = [analysisAt(1, "running")];
  await page.goto("/quality");
  await expect(page.getByText(/^Analysing now:/)).toBeVisible();
  // The server says a pass runs, so there is nothing to start: the offer
  // comes from its answer, not from what this page remembers doing.
  await expect(
    page.getByRole("button", { name: "Analyse again" }),
  ).toHaveCount(0);
});

test("asks for the header again when the list is filtered", async ({
  page,
}) => {
  const state = await serveQuality(page);
  state.analyses = [analysisAt(0, "idle"), analysisAt(2, "idle")];
  await page.goto("/quality");
  await expect(page.getByText("0 of 3 lossless files analysed")).toBeVisible();
  await page
    .getByRole("list", { name: "Verdicts" })
    .getByRole("link", { name: /No sign of a lossy source/ })
    .click();
  // At rest the summary is not polled, so only the filter can have asked.
  await expect(page.getByText("2 of 3 lossless files analysed")).toBeVisible();
});

test("asks for the header again on every visit", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name === "mobile", "walks the desktop sidebar");
  const state = await serveQuality(page);
  state.analyses = [analysisAt(0, "idle"), analysisAt(2, "idle")];
  await page.goto("/quality");
  await expect(page.getByText("0 of 3 lossless files analysed")).toBeVisible();
  // Away and back inside the cache's thirty seconds: an answer held from the
  // first visit would contradict the rows drawn on the second.
  const sidebar = page.getByRole("navigation");
  await sidebar.getByRole("link", { name: "Queue" }).click();
  await expect(page.getByRole("heading", { name: "Queue" })).toBeVisible();
  // The rail's entry opens the first section this owner may use, and the
  // hub's menu the one this test is about.
  await page.locator(".sidebar").getByRole("link", { name: "Maintenance" }).click();
  await page
    .getByRole("navigation", { name: "Maintenance sections" })
    .getByRole("link", { name: "Quality" })
    .click();
  await expect(page.getByText("2 of 3 lossless files analysed")).toBeVisible();
});
