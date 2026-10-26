// The Maintenance hub and its sections, and the missing files (RFC-021).

import { expect, test } from "@playwright/test";

import {
  chooseLightTheme,
  expectNoWcagViolations,
} from "./fixtures/accessibility";
import { resetAndMockApi } from "./fixtures/api";
import { library, missingTrack, session } from "./fixtures/catalogue";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

/**
 * The sections of Maintenance, in the order its menu offers them, with the
 * heading each opens on. The pages draw their heading whatever their lists
 * answer, so these tests need none of the lists served.
 */
const MAINTENANCE_SECTIONS = [
  { link: "Needs attention", path: "/maintenance/attention", heading: "Needs attention" },
  { link: "Missing files", path: "/maintenance/missing", heading: "Missing files" },
  { link: "Quality", path: "/maintenance/quality", heading: "Quality" },
];

/**
 * "Needs attention", "Missing files" and "Quality" are one rail entry,
 * Maintenance, and each is a section of it: opened from the rail on a
 * desktop and from the More sheet on a phone, each at its own address,
 * marked current in the menu, with Back walking between them.
 */
test("opens every section of Maintenance from its menu", async ({
  page,
}, testInfo) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  for (const name of ["Needs attention", "Missing files", "Quality"]) {
    await expect(
      page.locator(".sidebar, .mobile-navigation").getByRole("link", { name }),
      name,
    ).toHaveCount(0);
  }
  if (testInfo.project.name === "mobile") {
    await page
      .locator(".mobile-navigation")
      .getByRole("button", { name: "More" })
      .click();
    await page
      .getByRole("dialog")
      .getByRole("link", { name: "Maintenance" })
      .click();
  } else {
    await page
      .locator(".sidebar")
      .getByRole("link", { name: "Maintenance" })
      .click();
  }

  // The entry opens the first section this owner may use.
  await expect(page).toHaveURL(
    (url) => url.pathname === "/maintenance/attention",
  );
  const menu = page.getByRole("navigation", { name: "Maintenance sections" });
  await expect(menu.getByRole("link")).toHaveText(
    MAINTENANCE_SECTIONS.map((section) => section.link),
  );
  if (testInfo.project.name === "desktop") {
    // The rail's entry stays current in every section.
    await expect(
      page.locator(".sidebar a[data-status='active']"),
    ).toHaveText(["Maintenance"]);
  }

  for (const section of [...MAINTENANCE_SECTIONS.slice(1), MAINTENANCE_SECTIONS[0]]) {
    await menu.getByRole("link", { name: section.link, exact: true }).click();
    await expect(page).toHaveURL((url) => url.pathname === section.path);
    await expect(
      page.getByRole("heading", { name: section.heading, exact: true }),
    ).toBeVisible();
    await expect(menu.locator('a[aria-current="page"]')).toHaveText([
      section.link,
    ]);
  }
  await expectNoWcagViolations(page);

  // Back to the section opened before the last.
  await page.goBack();
  await expect(page).toHaveURL((url) => url.pathname === "/maintenance/quality");
  await expect(
    page.getByRole("heading", { name: "Quality", exact: true }),
  ).toBeVisible();
});

/**
 * The three old addresses arrive at their sections, with the open list or the
 * verdict filter they carried, and the redirect replaces its own history
 * entry.
 */
test("lands every old address on its section of Maintenance", async ({
  page,
}) => {
  for (const [address, path, search, heading] of [
    ["/attention?category=missing_year", "/maintenance/attention", "category=missing_year", "Needs attention"],
    ["/missing", "/maintenance/missing", "", "Missing files"],
    ["/quality?verdict=upsampled", "/maintenance/quality", "verdict=upsampled", "Quality"],
  ]) {
    await page.goto(address);
    await expect(page).toHaveURL(
      (url) => url.pathname === path && url.search === (search ? `?${search}` : ""),
    );
    await expect(
      page.getByRole("heading", { name: heading, exact: true }),
    ).toBeVisible();
  }

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await page.goto("/quality");
  await expect(page).toHaveURL((url) => url.pathname === "/maintenance/quality");
  await page.goBack();
  await expect(page).toHaveURL((url) => url.pathname === "/");
});

/**
 * Each section keeps the rule its rail entry had. A listener who is not an
 * administrator may only read the quality of the files, so that is the only
 * section their menu shows and the one the entry opens.
 */
test("shows a listener only the Maintenance sections theirs to use", async ({
  page,
}) => {
  await page.route("**/api/v2/web/auth/refresh", (route) =>
    route.fulfill({
      json: { ...session, user: { ...session.user, role: "user" } },
    }),
  );
  mock.libraries = [{ ...library("library-1", "Ma musique"), role: "listener" }];
  await page.goto("/maintenance");
  await expect(page).toHaveURL((url) => url.pathname === "/maintenance/quality");
  await expect(
    page.getByRole("heading", { name: "Quality", exact: true }),
  ).toBeVisible();
  await expect(
    page
      .getByRole("navigation", { name: "Maintenance sections" })
      .getByRole("link"),
  ).toHaveText(["Quality"]);
});

/**
 * An administrator who only listens in this library still purges what the
 * scans lost (RFC-021), and does not decide what needs attention (RFC-015).
 */
test("shows an administrator listening in a library the missing files", async ({
  page,
}) => {
  mock.libraries = [{ ...library("library-1", "Ma musique"), role: "listener" }];
  await page.goto("/maintenance");
  await expect(page).toHaveURL((url) => url.pathname === "/maintenance/missing");
  await expect(
    page
      .getByRole("navigation", { name: "Maintenance sections" })
      .getByRole("link"),
  ).toHaveText(["Missing files", "Quality"]);
});

for (const section of MAINTENANCE_SECTIONS) {
  test(`keeps the Maintenance menu readable in the light on ${section.link}`, async ({
    page,
  }) => {
    await chooseLightTheme(page);
    await page.goto(section.path);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "default");
    await expect(
      page.getByRole("heading", { name: section.heading, exact: true }),
    ).toBeVisible();
    await expectNoWcagViolations(page);
  });
}

/**
 * "Missing files" (RFC-021 decision 4): what the scans can no longer find, what
 * purging each would cost, and a purge that takes two clicks and only what was
 * chosen. A purge is not reversible, so the first click only arms it.
 */
test("lists the missing files and purges only what was chosen", async ({
  page,
}) => {
  mock.missingTracks = [missingTrack(1, "First light"), missingTrack(2, "Second wind")];
  await page.goto("/missing");

  await expect(
    page.getByRole("heading", { name: "Missing files" }),
  ).toBeVisible();
  await expect(
    page.getByText("2 files the scans can no longer find"),
  ).toBeVisible();
  await expect(page.getByText("Gone/2 Second wind.flac")).toBeVisible();
  await expect(
    page.getByText("2 in playlists · 1 favourites · 6 plays"),
  ).toBeVisible();
  await expectNoWcagViolations(page);

  await page.getByRole("checkbox", { name: "Select First light" }).check();
  const purge = page.getByRole("button", { name: "Purge the selected (1)" });
  await purge.click();
  // Armed, not sent.
  await expect(
    page.getByRole("button", { name: "Confirm: purge 1" }),
  ).toBeVisible();
  expect(mock.purgeAsks).toEqual([]);

  await page.getByRole("button", { name: "Confirm: purge 1" }).click();
  await expect(page.getByText("1 track purged.")).toBeVisible();
  expect(mock.purgeAsks).toEqual([{ track_ids: ["missing-1"] }]);
  // Out of the missing files, and into the purged ones it can come back from.
  const listed = page.locator("ul.missing-list").first();
  await expect(listed.getByText("Gone/1 First light.flac")).toHaveCount(0);
  await expect(listed.getByText("Gone/2 Second wind.flac")).toBeVisible();
  await expect(
    page
      .getByRole("region", { name: "Purged, still restorable (1)" })
      .getByText("Gone/1 First light.flac"),
  ).toBeVisible();
});

/** Purging is a curator's: a listener is told so and asks for nothing. */
test("keeps the missing files from a listener", async ({ page }) => {
  const asked: string[] = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (path.includes("/unavailable-tracks")) asked.push(path);
  });
  // A listener who is not an administrator either: the default session is
  // one, and an administrator may see the page whatever their library role.
  await page.route("**/api/v2/web/auth/refresh", (route) =>
    route.fulfill({
      json: { ...session, user: { ...session.user, role: "user" } },
    }),
  );
  mock.libraries = [{ ...library("library-1", "Ma musique"), role: "listener" }];
  mock.missingTracks = [missingTrack(1, "First light")];
  await page.goto("/missing");
  await expect(
    page.getByText(
      "Only the owners and managers of this library, and administrators, see its missing files.",
    ),
  ).toBeVisible();
  await expect(page.getByText("Gone/1 First light.flac")).toHaveCount(0);
  expect(asked).toEqual([]);
});

/**
 * The server grants the page to the instance's administrators whatever their
 * role in the library (RFC-021 decision 4), so the client must not hide it
 * from an administrator who only listens to it.
 */
test("shows the missing files to an administrator who only listens", async ({
  page,
}) => {
  mock.libraries = [{ ...library("library-1", "Ma musique"), role: "listener" }];
  mock.missingTracks = [missingTrack(1, "First light")];
  await page.goto("/missing");
  await expect(page.getByText("Gone/1 First light.flac")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Purge them all" }),
  ).toBeVisible();
});

/**
 * A purge can be taken back while the server keeps it (RFC-021, undoing a
 * purge): the purged track leaves the missing files for a list of its own,
 * says until when it can come back, and restoring it puts it back.
 */
test("takes a purge back while the server still keeps it", async ({ page }) => {
  mock.missingTracks = [missingTrack(1, "First light"), missingTrack(2, "Second wind")];
  await page.goto("/missing");
  await page.getByRole("checkbox", { name: "Select First light" }).check();
  await page.getByRole("button", { name: "Purge the selected (1)" }).click();
  await page.getByRole("button", { name: "Confirm: purge 1" }).click();
  await expect(
    page.getByText(/1 track purged\. It can be restored until/),
  ).toBeVisible();
  const held = page.getByRole("region", {
    name: "Purged, still restorable (1)",
  });
  await expect(held.getByText("Gone/1 First light.flac")).toBeVisible();
  await expect(held.getByText(/deleted for good after/)).toBeVisible();
  await expectNoWcagViolations(page);

  await held.getByRole("button", { name: "Restore First light" }).click();
  await expect(page.getByText("1 track restored.")).toBeVisible();
  expect(mock.restoreAsks).toEqual([{ track_ids: ["missing-1"] }]);
  await expect(
    page.getByRole("region", { name: /Purged, still restorable/ }),
  ).toHaveCount(0);
  await expect(page.getByText("Gone/1 First light.flac")).toBeVisible();
});
