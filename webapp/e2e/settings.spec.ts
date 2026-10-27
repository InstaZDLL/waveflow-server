// The Settings hub, its sections, the account's clients and Subsonic password.

import { expect, test } from "@playwright/test";

import {
  chooseLightTheme,
  expectNoWcagViolations,
} from "./fixtures/accessibility";
import { resetAndMockApi } from "./fixtures/api";
import { library, session } from "./fixtures/catalogue";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

/**
 * The sections of Settings, in the order the menu offers them, with the
 * heading each one opens on. Shared by the tests below, which walk the whole
 * menu: a section added to the hub and not here fails them by count.
 */
const SETTINGS_SECTIONS = [
  { link: "Account", path: "/settings", heading: "Account" },
  { link: "Playback", path: "/settings/playback", heading: "Playback" },
  {
    link: "Scrobbling",
    path: "/settings/scrobbling",
    heading: "External scrobbling",
  },
  { link: "Shares", path: "/settings/shares", heading: "Shares" },
  { link: "Upload", path: "/settings/upload", heading: "Upload to Ma musique" },
  { link: "Admin", path: "/settings/admin", heading: "Administration" },
];

/**
 * The rail carried an "Account" group of five entries — Shares, Settings,
 * Scrobbling, Upload and Admin. One Settings entry replaces it, in the rail's
 * foot beside the account, and the five are the Settings page's sections.
 *
 * Asked of the longest rail any account sees: an administrator owning a
 * library that takes files, for whom every one of the five used to show.
 */
test("gathers the account's places behind one Settings entry", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "the rail is desktop-only");
  mock.libraries = [
    { ...library("library-1", "Ma musique"), accepts_uploads: true },
  ];
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  const sidebar = page.locator(".sidebar");
  // Drawn in full before anything is counted: the Maintenance entry is gated
  // on what the account may do in the library, and a count taken before it
  // arrives proves nothing.
  await expect(sidebar.locator('a[href="/maintenance"]')).toHaveCount(1);

  await expect(sidebar.locator('.nav-group[data-group="account"]')).toHaveCount(
    0,
  );
  await expect(sidebar.locator(".nav-label")).toHaveText([
    "Library",
    "Listening",
  ]);
  const settings = sidebar.getByRole("link", { name: "Settings" });
  await expect(settings).toHaveCount(1);
  await expect(page.locator(".sidebar-footer").getByRole("link")).toHaveText([
    "Settings",
  ]);
  for (const href of [
    "/shares",
    "/upload",
    "/admin",
    "/settings/scrobbling",
    "/settings/shares",
    "/settings/upload",
    "/settings/admin",
  ]) {
    await expect(sidebar.locator(`a[href="${href}"]`), href).toHaveCount(0);
  }

  await settings.click();
  await expect(page).toHaveURL(/\/settings$/);
  await expect(
    page.getByRole("heading", { name: "Account", exact: true }),
  ).toBeVisible();
});

/**
 * Every section opens from the hub's own menu, on a desktop from the rail's
 * Settings entry and on a phone from the More sheet's — the phone has no
 * other way in, so this is also the proof that nothing became unreachable
 * there when the sheet lost four entries.
 *
 * Each section is its own address, marked as the current one in the menu,
 * and Back walks the sections in the order they were opened.
 */
test("opens every section of Settings from its menu", async ({
  page,
}, testInfo) => {
  mock.libraries = [
    { ...library("library-1", "Ma musique"), accepts_uploads: true },
  ];
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  if (testInfo.project.name === "mobile") {
    await page
      .locator(".mobile-navigation")
      .getByRole("button", { name: "More" })
      .click();
    await page
      .getByRole("dialog")
      .getByRole("link", { name: "Settings" })
      .click();
  } else {
    await page
      .locator(".sidebar")
      .getByRole("link", { name: "Settings" })
      .click();
  }

  const menu = page.getByRole("navigation", { name: "Settings sections" });
  await expect(menu.getByRole("link")).toHaveText(
    SETTINGS_SECTIONS.map((section) => section.link),
  );
  // The order is reversed so that every click moves the open section, Account
  // included, and so that Back has a different section to return to each time.
  const visited: typeof SETTINGS_SECTIONS = [];
  for (const section of [...SETTINGS_SECTIONS.slice(1), SETTINGS_SECTIONS[0]]) {
    await menu.getByRole("link", { name: section.link, exact: true }).click();
    await expect(page).toHaveURL((url) => url.pathname === section.path);
    await expect(
      page.getByRole("heading", { name: section.heading, exact: true }),
    ).toBeVisible();
    await expect(menu.locator('a[aria-current="page"]')).toHaveText([
      section.link,
    ]);
    visited.push(section);
  }
  // The menu is new, so it is audited in the dark theme here and in the light
  // one below, open on a short section and on a long one.
  await expectNoWcagViolations(page);

  if (testInfo.project.name === "mobile") {
    // A list two to a row, not a strip running off the side of the phone.
    const overflow = await page.evaluate(
      () =>
        document.documentElement.scrollWidth -
        document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(1);
    const columns = await menu.evaluate(
      (node) => getComputedStyle(node).gridTemplateColumns.split(" ").length,
    );
    expect(columns).toBe(2);
  }

  // Back returns to the section opened before, as a page would.
  visited.pop();
  const previous = visited.pop();
  await page.goBack();
  await expect(page).toHaveURL((url) => url.pathname === previous?.path);
  await expect(
    page.getByRole("heading", { name: previous?.heading, exact: true }),
  ).toBeVisible();
});

/**
 * Every address the moved pages had still arrives at its section. The three
 * old ones are redirects, and the redirect replaces its own history entry:
 * Back from the section leaves for where the visitor was before, rather than
 * landing on an address that would only send them forward again.
 */
test("lands every old address on its section of Settings", async ({ page }) => {
  mock.libraries = [
    { ...library("library-1", "Ma musique"), accepts_uploads: true },
  ];
  for (const [address, path, heading] of [
    ["/settings", "/settings", "Account"],
    ["/settings/scrobbling", "/settings/scrobbling", "External scrobbling"],
    ["/shares", "/settings/shares", "Shares"],
    ["/upload", "/settings/upload", "Upload to Ma musique"],
    ["/admin", "/settings/admin", "Administration"],
  ]) {
    await page.goto(address);
    await expect(page).toHaveURL((url) => url.pathname === path);
    await expect(
      page.getByRole("heading", { name: heading, exact: true }),
    ).toBeVisible();
  }

  // The folder an upload was started from survives the move.
  await page.goto("/upload?folder=folder-1");
  await expect(page).toHaveURL(
    (url) =>
      url.pathname === "/settings/upload" &&
      url.searchParams.get("folder") === "folder-1",
  );

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await page.goto("/shares");
  await expect(page).toHaveURL((url) => url.pathname === "/settings/shares");
  await page.goBack();
  await expect(page).toHaveURL((url) => url.pathname === "/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
});

/**
 * The gates the rail entries had moved with them onto the menu: someone who
 * is not an administrator sees no Admin section, and someone who may not
 * upload sees no Upload one. Neither address opens for them either.
 */
test("hides Admin and Upload from the Settings menu of an account without them", async ({
  page,
}) => {
  await page.route("**/api/v2/web/auth/refresh", (route) =>
    route.fulfill({
      json: { ...session, user: { ...session.user, role: "user" } },
    }),
  );
  mock.libraries = [
    {
      ...library("library-1", "Ma musique"),
      accepts_uploads: true,
      role: "listener",
    },
  ];
  await page.goto("/settings");
  await expect(
    page.getByRole("heading", { name: "Account", exact: true }),
  ).toBeVisible();
  const menu = page.getByRole("navigation", { name: "Settings sections" });
  await expect(menu.getByRole("link")).toHaveText([
    "Account",
    "Playback",
    "Scrobbling",
    "Shares",
  ]);

  for (const address of ["/admin", "/settings/admin"]) {
    await page.goto(address);
    await expect(page).toHaveURL((url) => url.pathname === "/");
    await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  }
});

// One test per section rather than one walking all six: each audit takes a
// few seconds, and six in a row overran the timeout under a full parallel run.
for (const section of SETTINGS_SECTIONS) {
  test(`keeps the Settings menu readable in the light on ${section.link}`, async ({
    page,
  }) => {
    mock.libraries = [
      { ...library("library-1", "Ma musique"), accepts_uploads: true },
    ];
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
 * The clients holding a credential on this account, which nothing showed
 * before.
 *
 * The material existed and was reachable only by the instance administrator,
 * buried in a disclosure on a screen an ordinary account cannot open. So a
 * listener could not find out which devices were reading their library, let
 * alone stop one.
 *
 * A revoked credential is left out rather than struck through: this list
 * answers who **can** read the library, and one that cannot is not an answer
 * to that question. The administration screen keeps the full history.
 */
test("shows the clients that can read the library, and revokes one", async ({
  page,
}) => {
  await page.goto("/settings");

  // Located by its heading rather than by its position. `.last()` named this
  // panel until a second self-scoped panel was added below it, and a locator
  // that depends on the order of the page silently follows whatever lands at
  // the end.
  const clients = page
    .locator(".settings-panel")
    .filter({ has: page.getByRole("heading", { name: "Registered clients" }) });
  await expect(
    clients.getByRole("heading", { name: "Registered clients" }),
  ).toBeVisible();
  // One of the three: the second is revoked and the third is past its
  // expiry, and neither can read the library any more.
  await expect(clients.getByRole("listitem")).toHaveCount(1);
  await expect(clients.getByText("the kitchen speaker")).toBeVisible();
  await expect(clients.getByText("a laptop that is gone")).toHaveCount(0);
  await expect(clients.getByText("a tablet whose year ran out")).toHaveCount(0);

  // Registering one shows the secret, once, and the list follows.
  await clients.getByLabel("What it is for").fill("my phone");
  await clients.getByRole("button", { name: "Register" }).click();
  await expect(
    page.getByText("wfapi_shown_once_and_never_again"),
  ).toBeVisible();
  await expect(clients.getByRole("listitem")).toHaveCount(2);

  // And revoking stops one. The row goes, because the row is the answer to
  // "who can read my library".
  await clients
    .getByRole("button", { name: "Revoke: the kitchen speaker" })
    .click();
  await expect(clients.getByText("the kitchen speaker")).toHaveCount(0);
  await expect(clients.getByRole("listitem")).toHaveCount(1);

  // **And a second press while the registration is out sends nothing.** Two
  // registrations mint two credentials and only one secret reaches the
  // screen: the other is a credential on this library whose secret nobody
  // holds. This panel shipped without the guard; the one below it has the
  // same one now, for the same reason.
  let release = () => {};
  mock.clientGate = new Promise<void>((resolve) => {
    release = resolve;
  });
  mock.clientWrites = [];
  const register = clients.getByRole("button", { name: "Register" });
  await clients.getByLabel("What it is for").fill("a slow tablet");
  await register.click();
  await expect.soft(register).toBeDisabled();
  await register.click({ force: true });
  await register.click({ force: true });
  release();
  await expect(register).toBeEnabled();
  expect(mock.clientWrites).toEqual(["POST"]);
});

/**
 * The account changes its own Subsonic password, and switches it off.
 *
 * That password is typed into third-party clients, on phones, which is the
 * ordinary way an application password leaks. Until this panel its owner could
 * do neither — both server routes were administrative — so answering "my key
 * has leaked" meant finding an operator.
 */
test("changes the Subsonic password, then switches it off", async ({
  page,
}) => {
  await page.goto("/settings");

  const subsonic = page
    .locator(".settings-panel")
    .filter({ has: page.getByRole("heading", { name: "Subsonic password" }) });
  await expect(subsonic).toBeVisible();
  // Set and never changed, so one date and not two.
  await expect(subsonic.getByText(/^set /)).toBeVisible();
  await expect(subsonic.getByText(/last changed/)).toHaveCount(0);

  // **The key moves with the password**, and it is shown once. A leaked
  // `wfsk_` key authenticates on its own, so a rotation that kept it would
  // answer nothing.
  await subsonic.getByLabel("New password").fill("a long enough replacement");
  await subsonic.getByRole("button", { name: "Change it" }).click();
  await expect(page.getByText("wfsk_shown_once_only")).toBeVisible();
  await expect(subsonic.getByText(/last changed/)).toBeVisible();

  // **Asked twice.** The account cannot put this credential back without an
  // operator, so the first press only arms the second. `exact` because
  // "Yes, switch it off" contains "Switch it off", and the default substring
  // match would make every locator here ambiguous.
  const off = subsonic.getByRole("button", {
    name: "Switch it off",
    exact: true,
  });
  const reallyOff = subsonic.getByRole("button", {
    name: "Yes, switch it off",
  });
  await off.click();
  await expect(off).toHaveCount(0);
  // Backing out leaves the credential exactly as it was.
  await subsonic.getByRole("button", { name: "Cancel" }).click();
  await expect(off).toBeVisible();
  await expect(reallyOff).toHaveCount(0);
  await expect(subsonic.getByText(/^set /)).toBeVisible();

  // Switching it off leaves the panel saying the account holds none — and
  // offering nothing, because creating one opens the facade to an account and
  // that is the operator's decision.
  await off.click();
  await reallyOff.click();
  await expect(
    subsonic.getByText(
      "You have no Subsonic password, so no Subsonic client can read your library. Ask the administrator for one.",
    ),
  ).toBeVisible();
  await expect(subsonic.getByLabel("New password")).toHaveCount(0);
  await expect(subsonic.getByRole("button", { name: "Change it" })).toHaveCount(
    0,
  );
});

/**
 * A short password is refused before it is sent.
 *
 * The server answers 422 under twelve characters, but that arrives after the
 * person has committed to the password they chose — and the panel would have
 * nothing to say about which of the two failures it was.
 */
test("refuses a Subsonic password under twelve characters", async ({
  page,
}) => {
  await page.goto("/settings");

  const subsonic = page
    .locator(".settings-panel")
    .filter({ has: page.getByRole("heading", { name: "Subsonic password" }) });
  await subsonic.getByLabel("New password").fill("abcdefghijk");
  await subsonic.getByRole("button", { name: "Change it" }).click();
  await expect(subsonic.getByText("Use at least 12 characters.")).toBeVisible();
  // Nothing was sent: the credential still reads as never changed.
  await expect(subsonic.getByText(/last changed/)).toHaveCount(0);
  await expect(page.getByText("wfsk_shown_once_only")).toHaveCount(0);
});

/**
 * A second press while the rotation is out sends nothing.
 *
 * **This one is about a secret, not about tidiness.** Each rotation mints a key
 * and stores its hash, so two in flight leave the second one's hash stored
 * while the panel may have settled on the first one's key — a `wfsk_` key
 * already dead, shown for a credential whose whole point is that it is shown
 * once. The person copies it into their clients and the working key is never
 * shown again.
 */
test("sends one rotation however many times the button is pressed", async ({
  page,
}) => {
  let release = () => {};
  mock.subsonicGate = new Promise<void>((resolve) => {
    release = resolve;
  });

  await page.goto("/settings");
  const subsonic = page
    .locator(".settings-panel")
    .filter({ has: page.getByRole("heading", { name: "Subsonic password" }) });
  const change = subsonic.getByRole("button", { name: "Change it" });

  await subsonic.getByLabel("New password").fill("a long enough replacement");
  await change.click();
  // **Soft**, which is the only `expect.soft` in this file and earns it: the
  // disabled state is the mechanism and the request count below is the
  // consequence, and a hard failure here would stop the test before the
  // consequence was ever measured. Removing the guard has to show both.
  await expect.soft(change).toBeDisabled();
  await change.click({ force: true });
  await change.click({ force: true });

  release();
  await expect(page.getByText("wfsk_shown_once_only")).toBeVisible();
  await expect(change).toBeEnabled();
  expect(mock.subsonicWrites).toEqual(["PUT"]);
});

test("keeps settings and the sign-in screen readable in the light", async ({
  page,
}) => {
  await chooseLightTheme(page);
  await page.goto("/settings");
  await expect(
    page.getByRole("heading", { name: "Account", exact: true }),
  ).toBeVisible();
  await expectNoWcagViolations(page);

  await page.context().clearCookies();
  await page.route("**/api/v2/**", (route) =>
    route.fulfill({ status: 401, json: { error: "unauthorized" } }),
  );
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Sign in" })).toBeVisible();
  await expectNoWcagViolations(page);
});
