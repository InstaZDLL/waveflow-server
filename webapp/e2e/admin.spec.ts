// The administration page: scans, tokens, members and a library's watch.

import { expect, test } from "@playwright/test";

import { resetAndMockApi } from "./fixtures/api";
import { library } from "./fixtures/catalogue";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

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
  mock.tokensFail = true;
  await page.goto("/admin");
  await page.getByRole("button", { name: "API tokens for listener" }).click();

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
  mock.scanFails = true;
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
  mock.scanDrops = true;
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
    if (path.endsWith("/members") || path.endsWith("/settings")) {
      asked.push(path);
    }
  });

  // Two libraries, because one cannot tell the two layouts apart: with a single
  // library, "list then panel" and "list, then every panel" produce the same
  // rows. The defect only shows from the second library on.
  mock.libraries = [
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
  // below it, which reads as belonging to whatever is above them. Its name,
  // folder and deletion (RFC-021) follow, still before the next library.
  const rows = page.locator(".admin-panel .resource-list > li");
  await expect(rows.nth(0)).toContainText("Ma musique");
  await expect(rows.nth(1)).toContainText("Who may see this library");
  await expect(rows.nth(2)).toContainText("Name, folder and deletion");
  await expect(rows.nth(3)).toContainText("Les enfants");
  await expect(rows.nth(4)).toContainText("Who may see this library");
  await expect(rows.nth(5)).toContainText("Name, folder and deletion");
  await expect(rows).toHaveCount(6);
  // Neither panel asks for anything until it is opened: the folder on the
  // server is fetched for the one library somebody is looking at.
  expect(asked).toEqual([]);

  await disclosure.click();
  await expect(page.getByText("guest")).toBeVisible();
  expect(asked).toEqual(["/api/v2/libraries/library-1/members"]);

  // The owner is shown and offers no role control: the route refuses `owner`
  // outright, so a select here would be offering a refusal.
  await expect(page.getByText("owner, and stays one")).toBeVisible();
  await expect(
    page.getByRole("combobox", { name: "Role: listener" }),
  ).toHaveCount(0);
  await expect(page.getByRole("combobox", { name: "Role: guest" })).toHaveValue(
    "listener",
  );
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
  mock.membersFail = true;
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

/** A library's watch is an administrator's switch beside its links setting. */
test("switches a library's watch off from its settings", async ({ page }) => {
  const updates: unknown[] = [];
  await page.route("**/api/v2/libraries/library-1", async (route) => {
    if (route.request().method() !== "PATCH") {
      await route.fallback();
      return;
    }
    updates.push(route.request().postDataJSON());
    await route.fulfill({ json: { scan_id: null } });
  });
  await page.goto("/admin");
  await page.getByRole("button", { name: "Name, folder and deletion" }).click();
  const watch = page.getByRole("checkbox", { name: /Watch the folder/ });
  await expect(watch).toBeChecked();
  await watch.uncheck();
  await page.getByRole("button", { name: "Save" }).click();
  await expect.poll(() => updates).toEqual([{ watch: false }]);
});
