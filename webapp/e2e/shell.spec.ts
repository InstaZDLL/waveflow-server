// The studio shell: navigation, the mobile bar, the logo kit, the manifest.

import { expect, test } from "@playwright/test";

import { expectNoWcagViolations } from "./fixtures/accessibility";
import { resetAndMockApi } from "./fixtures/api";
import { library, session } from "./fixtures/catalogue";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

test("renders the studio shell and persists appearance preferences", async ({
  page,
}, testInfo) => {
  await page.goto("/");

  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(page.getByText("Post", { exact: true })).toBeVisible();
  await expect(page.getByText("Vespertine", { exact: true })).toBeVisible();

  // **The pickers live on the Settings page now**, not in the sidebar footer
  // and not in the mobile header. They are settings and not destinations, and
  // they sat under thirteen navigation entries; there is no copy left behind,
  // because two ways to ask one question is the thing that move was about.
  // Settings opens on its Account section, which holds them.
  await page.goto("/settings");
  await expect(
    page.getByRole("heading", { name: "Account", exact: true }),
  ).toBeVisible();
  const preferences = page.locator(".settings-panel").first();
  const theme = preferences.getByRole("radiogroup", { name: "Theme" });
  await theme.getByRole("radio", { name: "Midnight light" }).check();
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("waveflow.theme")))
    .toBe("midnight-light");

  const language = preferences.getByLabel("Language");
  await language.selectOption("fr");
  await expect(
    page.getByRole("link", { name: "Recherche" }).first(),
  ).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("lang", "fr");
  // And nowhere else: a picker still reachable from the corridor would mean
  // the move had added a place rather than chosen one.
  await expect(page.locator(".sidebar").getByLabel("Thème")).toHaveCount(0);
  await expect(page.locator(".mobile-header").getByLabel("Thème")).toHaveCount(
    0,
  );

  if (testInfo.project.name === "mobile") {
    await expect(page.locator(".mobile-navigation")).toBeVisible();
    const overflow = await page.evaluate(
      () =>
        document.documentElement.scrollWidth -
        document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(1);
  } else {
    await expect(page.locator(".sidebar")).toBeVisible();
  }
});

test("keeps navigation and player controls within their own space", async ({
  page,
}, testInfo) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(page.locator(".player")).toBeVisible();

  if (testInfo.project.name === "mobile") {
    await expect(page.locator(".sidebar")).toBeHidden();
    const bounds = await page.evaluate(() => {
      const track = document
        .querySelector(".player-track")
        ?.getBoundingClientRect();
      const progress = document
        .querySelector(".player-progress")
        ?.getBoundingClientRect();
      const player = document.querySelector(".player")?.getBoundingClientRect();
      const navigation = document
        .querySelector(".mobile-navigation")
        ?.getBoundingClientRect();
      return {
        trackBottom: track?.bottom ?? 0,
        progressTop: progress?.top ?? 0,
        playerBottom: player?.bottom ?? 0,
        navigationTop: navigation?.top ?? 0,
        overflow: document.documentElement.scrollWidth - window.innerWidth,
      };
    });
    expect(bounds.progressTop).toBeGreaterThanOrEqual(bounds.trackBottom);
    expect(bounds.navigationTop).toBeGreaterThanOrEqual(bounds.playerBottom);
    expect(bounds.overflow).toBeLessThanOrEqual(1);
    await page.setViewportSize({ width: 320, height: 700 });
    const narrow = await page.evaluate(() => ({
      trackBottom:
        document.querySelector(".player-track")?.getBoundingClientRect()
          .bottom ?? 0,
      progressTop:
        document.querySelector(".player-progress")?.getBoundingClientRect()
          .top ?? 0,
      playerBottom:
        document.querySelector(".player")?.getBoundingClientRect().bottom ?? 0,
      navigationTop:
        document.querySelector(".mobile-navigation")?.getBoundingClientRect()
          .top ?? 0,
      overflow: document.documentElement.scrollWidth - window.innerWidth,
    }));
    expect(narrow.progressTop).toBeGreaterThanOrEqual(narrow.trackBottom);
    expect(narrow.navigationTop).toBeGreaterThanOrEqual(narrow.playerBottom);
    expect(narrow.overflow).toBeLessThanOrEqual(1);
  } else {
    const bounds = await page.evaluate(() => {
      const sidebar = document
        .querySelector(".sidebar")
        ?.getBoundingClientRect();
      const footer = document
        .querySelector(".sidebar-footer")
        ?.getBoundingClientRect();
      const navigation = document
        .querySelector(".primary-navigation")
        ?.getBoundingClientRect();
      return {
        sidebarBottom: sidebar?.bottom ?? 0,
        footerBottom: footer?.bottom ?? 0,
        navigationBottom: navigation?.bottom ?? 0,
      };
    });
    expect(bounds.footerBottom).toBeLessThanOrEqual(bounds.sidebarBottom);
    expect(bounds.navigationBottom).toBeLessThanOrEqual(bounds.footerBottom);
    await expect(
      page.locator(".sidebar-footer").getByRole("button", { name: "Sign out" }),
    ).toBeVisible();
    await page.setViewportSize({ width: 821, height: 650 });
    const compact = await page.evaluate(() => ({
      sidebarBottom:
        document.querySelector(".sidebar")?.getBoundingClientRect().bottom ?? 0,
      footerBottom:
        document.querySelector(".sidebar-footer")?.getBoundingClientRect()
          .bottom ?? 0,
      overflow: document.documentElement.scrollWidth - window.innerWidth,
    }));
    expect(compact.footerBottom).toBeLessThanOrEqual(compact.sidebarBottom);
    expect(compact.overflow).toBeLessThanOrEqual(1);
  }
});

test("shows one current navigation item and separates administration panels", async ({
  page,
}, testInfo) => {
  test.skip(
    testInfo.project.name !== "desktop",
    "sidebar layout is desktop-only",
  );
  // A section of Settings: the rail has no entry of its own for it any more,
  // and the one entry that leads there is the current one — in the rail's
  // foot, so the whole sidebar is counted and not only the rail.
  await page.goto("/settings/scrobbling");
  const current = page.locator(".sidebar a[data-status='active']");
  await expect(current).toHaveCount(1);
  await expect(current).toHaveText("Settings");

  const genres = page.locator(".primary-navigation").getByRole("link", {
    name: "Genres",
  });
  await genres.hover();
  const activeBackground = await current.evaluate(
    (node) => getComputedStyle(node).backgroundColor,
  );
  const hoverBackground = await genres.evaluate(
    (node) => getComputedStyle(node).backgroundColor,
  );
  expect(hoverBackground).not.toBe(activeBackground);

  await page.goto("/admin");
  await expect(page.locator(".admin-page > .stack .user-row")).toBeVisible();
  await expect(page.locator(".now-playing-panel")).toBeVisible();
  const gap = await page.evaluate(() => {
    const accounts = document.querySelector(".admin-page > .stack");
    const playing = document.querySelector(".now-playing-panel");
    return (
      (playing?.getBoundingClientRect().top ?? 0) -
      (accounts?.getBoundingClientRect().bottom ?? 0)
    );
  });
  expect(gap).toBeGreaterThanOrEqual(24);
});

/**
 * The rail holds every destination without scrolling at a laptop's 1366×768
 * and a desktop's 1440×900, and the Settings entry in its foot is in view and
 * clear of the player bar.
 *
 * Asked of the longest rail any account sees — an administrator owning a
 * library that takes files — and measured after the fonts have loaded, since
 * a face arriving late changes every row's height. Three things got it
 * there: the Account group became one Settings entry, three maintenance
 * pages became one Maintenance entry, and a short window draws a denser rail
 * (fluid.css). Before all three the rail was 1112 px tall in 456 px at
 * 1366×768, and in 588 px at 1440×900.
 */
test("fits the rail without scrolling at common desktop heights", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "the rail is desktop-only");
  mock.libraries = [
    { ...library("library-1", "Ma musique"), accepts_uploads: true },
  ];
  for (const [width, height] of [
    [1366, 768],
    [1440, 900],
  ]) {
    await page.setViewportSize({ width, height });
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
    const rail = page.locator(".primary-navigation");
    await expect(rail.locator('a[href="/maintenance"]')).toHaveCount(1);
    await page.evaluate(() => document.fonts.ready);
    const measured = await rail.evaluate((node) => ({
      scrollHeight: node.scrollHeight,
      clientHeight: node.clientHeight,
    }));
    testInfo.annotations.push({
      type: "rail",
      description: `${width}x${height}: scrollHeight ${measured.scrollHeight}, clientHeight ${measured.clientHeight}`,
    });
    expect(
      measured.scrollHeight,
      `the rail scrolls at ${width}x${height}`,
    ).toBeLessThanOrEqual(measured.clientHeight);
    // Dense, not cramped: every row of the sidebar stays a target of at
    // least 24 px (WCAG 2.5.8).
    const smallest = await page
      .locator(".sidebar")
      .evaluate((node) =>
        Math.min(
          ...[...node.querySelectorAll("a, button")].map(
            (target) => target.getBoundingClientRect().height,
          ),
        ),
      );
    expect(smallest, `targets at ${width}x${height}`).toBeGreaterThanOrEqual(
      24,
    );
    const settings = page
      .locator(".sidebar")
      .getByRole("link", { name: "Settings" });
    await expect(settings, `at ${width}x${height}`).toBeInViewport({
      ratio: 1,
    });
    // And not under the player bar, which runs across the rail's foot.
    const overlap = await settings.evaluate((link) => {
      const player = document.querySelector(".player")?.getBoundingClientRect();
      return link.getBoundingClientRect().bottom - (player?.top ?? 0);
    });
    expect(overlap, `at ${width}x${height}`).toBeLessThanOrEqual(0);
  }
});

test("offers a keyboard skip link and a localized not-found route", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await page.keyboard.press("Tab");
  const skip = page.getByRole("link", { name: "Skip to content" });
  await expect(skip).toBeFocused();
  await skip.press("Enter");
  await expect(page.locator("#main-content")).toBeFocused();

  await page.goto("/a-room-that-does-not-exist");
  await expect(
    page.getByRole("heading", { name: "This room is silent" }),
  ).toBeVisible();
  await expect(
    page.getByRole("link", { name: "Back to the library" }),
  ).toBeVisible();
});

test("has no automated WCAG A or AA violations", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expectNoWcagViolations(page);
});

test("keeps the light appearance readable", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("waveflow.theme", "default");
  });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "default");
  await expectNoWcagViolations(page);
});

/**
 * The mobile bar holds six slots and the client has sixteen screens. Until
 * 2026-09-15 the other eight were reachable by typing their address and by
 * nothing else: the sidebar that carries them is `display: none` below 820px.
 * The ninth is Songs, added with the listing itself, and the tenth is
 * Settings — which is where the theme and the language went when they stopped
 * living in the sidebar footer.
 * Seven of the eight had been in that state since lot A, and no test saw it —
 * every mobile case here navigates by `page.goto`.
 */
test("opens the rest of the client from the mobile bar", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "mobile", "about the phone bar");
  // An administrator of a library that takes files, so all eight are this
  // account's to see. The default fixture takes none, which left `Upload` out
  // of the list below while the comment on it claimed otherwise — and nothing
  // then proved the sheet ever shows an upload entry at all. Its absence in
  // the test beside this one is over-determined: that account is a listener
  // *and* its library is closed, so it would pass against a sheet that had
  // dropped the entry entirely.
  mock.libraries = [
    { ...library("library-1", "Ma musique"), accepts_uploads: true },
  ];
  await page.goto("/");

  const bar = page.locator(".mobile-navigation");
  // Six links and the seventh slot, on one row. The grid counted five for as
  // long as there were five, and a sixth primary entry added later had been
  // wrapping onto a second row ever since.
  await expect(bar.locator("a")).toHaveCount(6);
  const rows = await bar.evaluate(
    (element) =>
      new Set(
        [...element.children].map((child) =>
          Math.round(child.getBoundingClientRect().top),
        ),
      ).size,
  );
  expect(rows).toBe(1);

  // Asserted before it is clicked. Clicking a control that is not there fails
  // by timing out, and a timeout in a CI log says a test was slow rather than
  // that the seventh slot is missing.
  const more = bar.getByRole("button", { name: "More" });
  await expect(more).toBeVisible();
  await more.click();

  const sheet = page.getByRole("dialog");
  await expect(sheet).toBeVisible();
  // The twelve the bar cannot hold. `Radio` joined them with #217, `Folders`
  // with RFC-012's filed folders, `Offline` with RFC-022, `Statistics` with
  // RFC-024 and `Podcasts` with RFC-025. Shares, Scrobbling, Upload and Admin
  // left it when they became sections of Settings, and `Needs attention`
  // (RFC-015), `Missing files` (RFC-021) and `Quality` (RFC-016) when they
  // became sections of Maintenance — which is how eighteen became twelve.
  // This count is what said so: a new entry in the sidebar reaches the sheet,
  // and the sheet is pinned.
  for (const name of [
    "Artists",
    "Songs",
    "Genres",
    "Folders",
    "Maintenance",
    "Settings",
    "Random",
    "Recently played",
    "Statistics",
    "Offline",
    "Radio",
    "Podcasts",
  ]) {
    await expect(sheet.getByRole("link", { name })).toBeVisible();
  }
  // Counted as well as named: a sheet that also offered something nobody asked
  // for would satisfy every line above.
  await expect(sheet.getByRole("link")).toHaveCount(12);

  await expectNoWcagViolations(page);

  // Escape is the keyboard's way out, and it is the element's own rather than
  // anything written here — which is why the sheet is a <dialog>.
  await page.keyboard.press("Escape");
  await expect(sheet).toBeHidden();

  await bar.getByRole("button", { name: "More" }).click();
  await sheet.getByRole("link", { name: "Settings" }).click();
  await expect(
    page.getByRole("heading", { name: "Account", exact: true }),
  ).toBeVisible();
  // Closed on the way out: a sheet left open would sit over the page it just
  // reached.
  await expect(sheet).toBeHidden();
});

test("offers nothing in the sheet that the account may not reach", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "mobile", "about the phone bar");
  // A listener, in a library that takes no files: upload and admin are not
  // hidden by width, they are not this account's to see at all.
  await page.route("**/api/v2/web/auth/refresh", (route) =>
    route.fulfill({
      json: { ...session, user: { ...session.user, role: "user" } },
    }),
  );
  mock.libraries = [
    { ...library("library-1", "Ma musique"), role: "listener" },
  ];

  await page.goto("/");
  await page
    .locator(".mobile-navigation")
    .getByRole("button", { name: "More" })
    .click();

  const sheet = page.getByRole("dialog");
  // Settings is everyone's. What sits behind it is gated in its own menu, and
  // the test of that menu for this account is with the hub's.
  await expect(sheet.getByRole("link", { name: "Settings" })).toBeVisible();
  await expect(sheet.getByRole("link", { name: "Admin" })).toHaveCount(0);
  await expect(sheet.getByRole("link", { name: "Upload" })).toHaveCount(0);
  // Every member reads the quality of the files (RFC-016), so Maintenance
  // has a section for a listener; which ones is the hub's test.
  await expect(sheet.getByRole("link", { name: "Maintenance" })).toBeVisible();
  await expect(
    sheet.getByRole("link", { name: "Needs attention" }),
  ).toHaveCount(0);
  await expect(sheet.getByRole("link", { name: "Missing files" })).toHaveCount(
    0,
  );
});

/**
 * The icon is declared, and the declared icon is really there.
 *
 * Covered here rather than in the Rust suite, which runs against whatever
 * `build.rs` staged: a real client build when one exists, and a placeholder
 * holding nothing but `index.html` otherwise. The Rust CI never builds the
 * client, so asserting this there would pass on a developer's machine and fail
 * in CI for a reason that has nothing to do with the code. This project's
 * preview server holds the actual build.
 *
 * Both icons are the logo kit's, as its head snippet declares them: the `.ico`
 * sized for the browsers that read nothing else, and the SVG. Both carry the
 * kit's small drawing, the heavier stroke the guide asks for below 24px — a
 * flat Wave green W, where the old icon was five bars cut out of a gradient.
 *
 * What the server does with them — the file when the build has it, a JSON 404
 * when it does not, never the shell, which a browser cannot decode and asks
 * for again — belongs to the fallback and is pinned in `tests/service.rs`.
 */
test("declares the logo kit's icons, and serves the ones it declares", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "one viewport is enough");

  await page.goto("/");
  const icons = page.locator('link[rel="icon"]');
  await expect(icons).toHaveCount(2);
  await expect(icons.nth(0)).toHaveAttribute("href", "/favicon.ico");
  await expect(icons.nth(0)).toHaveAttribute("sizes", "48x48");
  await expect(icons.nth(1)).toHaveAttribute("href", "/favicon.svg");
  await expect(icons.nth(1)).toHaveAttribute("type", "image/svg+xml");

  const svg = await page.request.get("/favicon.svg");
  expect(svg.status()).toBe(200);
  expect(svg.headers()["content-type"]).toContain("image/svg+xml");
  const drawing = await svg.text();
  expect(drawing).toContain('fill="#10B981"');
  expect(drawing).not.toContain("Gradient");
  expect(drawing).not.toContain("<rect");

  const ico = await page.request.get("/favicon.ico");
  expect(ico.status()).toBe(200);
  // An ICO directory: reserved zero, then type 1, then the image count.
  const bytes = await ico.body();
  expect([...bytes.subarray(0, 4)]).toEqual([0, 0, 1, 0]);
  expect(bytes.readUInt16LE(4)).toBeGreaterThan(0);
});

/**
 * The sidebar's brand is the logo kit's horizontal logo, in the version the
 * guide gives for the background it sits on: the colour file on a light
 * theme, the dark file (mint and white) on a dark one. The theme is the
 * reader's preference rather than the system's, so the choice is made by the
 * root's `dark` class — and both themes are asked here, because a rule that
 * showed one file everywhere would pass on either alone.
 *
 * On a phone the same brand sits in the header instead of the rail; the
 * mobile project asks there.
 */
test("draws the brand from the logo kit, in the version for the theme", async ({
  page,
}, testInfo) => {
  const where =
    testInfo.project.name === "mobile" ? ".mobile-header" : ".sidebar";
  const brand = page.locator(`${where} .brand`);

  const expectLogo = async (file: string, hidden: string) => {
    const shown = brand.locator(`img[src="/brand/${file}"]`);
    await expect(shown).toBeVisible();
    await expect(brand.locator(`img[src="/brand/${hidden}"]`)).toBeHidden();
    // Really drawn, not a broken image the size of its attributes.
    await expect
      .poll(() =>
        shown.evaluate((image: HTMLImageElement) => image.naturalWidth),
      )
      .toBeGreaterThan(0);
    const drawn = await shown.evaluate((image) => {
      const style = getComputedStyle(image);
      return {
        width: image.getBoundingClientRect().width,
        animation: style.animationName,
        filter: style.filter,
      };
    });
    // The guide's minimum for the horizontal version, and none of what it
    // rules out: no pulsing, no glow or shadow.
    expect(drawn.width).toBeGreaterThanOrEqual(120);
    expect(drawn.animation).toBe("none");
    expect(drawn.filter).toBe("none");
    // The logo and the label beside it fit their row.
    expect(
      await brand.evaluate((link) => link.scrollWidth <= link.clientWidth),
    ).toBe(true);
  };

  await page.goto("/");
  await expect(page.locator("html")).toHaveClass(/\bdark\b/);
  await expectLogo(
    "waveflow-logo-horizontal-dark.svg",
    "waveflow-logo-horizontal-color.svg",
  );

  await page.evaluate(() => localStorage.setItem("waveflow.theme", "default"));
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "default");
  await expect(page.locator("html")).not.toHaveClass(/\bdark\b/);
  await expectLogo(
    "waveflow-logo-horizontal-color.svg",
    "waveflow-logo-horizontal-dark.svg",
  );
});

/**
 * The first-load screen is in the document the server sends, and it carries
 * the one vector file.
 *
 * Structural only: nothing here times anything. The floor and the fade are
 * pinned in `src/lib/splash.test.ts` against fake timers, because an assertion on
 * when the screen left would be an assertion about how fast the runner parsed
 * 276 kB of React.
 *
 * What this adds is the half a unit test cannot see: that the markup ships in
 * `index.html` rather than in the bundle — a splash inside the bundle could
 * only appear after the wait it exists to dress — and that it points at the
 * logo kit's symbol, in mint because this screen is always the dark
 * background. A hand-drawn copy of the mark is the defect this screen's own
 * issue was filed about. And that it holds still: the mark used to breathe
 * here, and the brand guide rules a pulsing logo out.
 */
test("ships the first-load screen in the document, drawn from the logo kit", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "one viewport is enough");

  // Read off the wire, before any script has run. Asking the rendered page
  // would be asking after the screen has already been taken away.
  const document_ = await page.request.get("/");
  const html = await document_.text();
  expect(html).toContain('id="splash"');
  expect(html).toContain('src="/brand/waveflow-symbol-mint.svg"');
  const inlineStyle = html.split("<style>")[1]?.split("</style>")[0] ?? "";
  expect(inlineStyle).toContain("#splash");
  expect(inlineStyle).not.toContain("animation");
  expect(inlineStyle).not.toContain("@keyframes");

  const logo = await page.request.get("/brand/waveflow-symbol-mint.svg");
  expect(logo.status()).toBe(200);
  expect(logo.headers()["content-type"]).toContain("image/svg+xml");
  // The kit's symbol: one flat path in mint, no gradient — so a file swapped
  // back to the old five bars fails here.
  const svg = await logo.text();
  expect(svg.match(/<path/g)).toHaveLength(1);
  expect(svg).toContain('fill="#34D399"');
  expect(svg).not.toContain("Gradient");
  expect(svg).not.toContain("<rect");

  await page.goto("/");
  // Gone from the document once the application is up — removed rather than
  // left transparent, because an element over the whole viewport still takes
  // a click at zero opacity.
  await expect(page.locator("#splash")).toHaveCount(0);
  await expect(page.locator("#main-content")).toBeVisible();

  // And on an address no route claims. This is the reason the dismissal sits
  // in the root route component rather than in the sign-in and shell
  // components: `notFoundComponent` belongs to the root and renders inside
  // neither of those, so a first visit to an unknown address would have kept
  // the mark until the ten-second ceiling.
  await page.goto("/no-such-screen");
  await expect(page.locator("#splash")).toHaveCount(0);
});

/**
 * The first-load screen leaves even when the client never arrives.
 *
 * `dismissSplash` runs from the bundle, so it cannot be what rescues a bundle
 * that fails to load: a reader would be left with a pulsing mark over the
 * whole viewport for as long as the tab stayed open — worse than a blank
 * page, which at least looks broken rather than looking like it is working.
 *
 * Eleven seconds of patience here, against a ten-second ceiling. It is the
 * slowest test in this file and it earns it: an untested fallback for a
 * failure nobody reproduces is the kind that turns out not to work on the one
 * day it matters.
 */
test("takes the first-load screen away even when the bundle never loads", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "one viewport is enough");

  // Every script the document asks for is refused, so React never mounts and
  // nothing in the bundle can clean up after it.
  await page.route("**/*.js", (route) => route.abort());
  await page.route("**/src/main.tsx", (route) => route.abort());

  await page.goto("/");
  // Up, and covering the page: this is the state the ceiling exists for.
  await expect(page.locator("#splash")).toBeVisible();

  // While it is up is the only moment the leaving state can be read on the
  // real element. Marked by hand rather than waited for: what is checked is
  // the rule, not the clock. Without `pointer-events: none` the layer keeps
  // taking clicks for the length of the fade — and for all of it under
  // `prefers-reduced-motion`, where it is already invisible.
  await page.locator("#splash").evaluate((element: HTMLElement) => {
    element.dataset.leaving = "true";
  });
  await expect(page.locator("#splash")).toHaveCSS("pointer-events", "none");
  await page.locator("#splash").evaluate((element: HTMLElement) => {
    delete element.dataset.leaving;
  });

  await expect(page.locator("#splash")).toHaveCount(0, { timeout: 11_000 });
  // What is revealed is an empty root — the page as it was before this screen
  // existed. The ceiling restores the old failure rather than inventing one.
  await expect(page.locator("#root")).toBeEmpty();
});

/**
 * RFC-022 decision 4: the client declares a manifest a phone can install from,
 * and every icon it names is served as an image.
 */
test("declares an install manifest whose icons are all served", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "one viewport is enough");
  await page.goto("/");
  await expect(page.locator('link[rel="manifest"]')).toHaveAttribute(
    "href",
    "/manifest.webmanifest",
  );
  await expect(page.locator('link[rel="apple-touch-icon"]')).toHaveAttribute(
    "href",
    "/apple-touch-icon.png",
  );
  const response = await page.request.get("/manifest.webmanifest");
  expect(response.status()).toBe(200);
  expect(response.headers()["content-type"]).toContain(
    "application/manifest+json",
  );
  const manifest = (await response.json()) as {
    start_url: string;
    display: string;
    theme_color: string;
    background_color: string;
    icons: Array<{ src: string; sizes: string; purpose: string }>;
  };
  expect(manifest.start_url).toBe("/");
  expect(manifest.display).toBe("standalone");
  // Wave green for both, as the logo kit's own manifest and head snippet set
  // them: the launch screen is the icon's tile, edge to edge.
  expect(manifest.theme_color).toBe("#10B981");
  expect(manifest.background_color).toBe("#10B981");
  await expect(page.locator('meta[name="theme-color"]')).toHaveAttribute(
    "content",
    "#10B981",
  );
  expect(manifest.icons.map((icon) => icon.sizes)).toEqual(
    expect.arrayContaining(["192x192", "512x512"]),
  );
  expect(manifest.icons.some((icon) => icon.purpose === "maskable")).toBe(true);
  for (const { src, sizes } of [
    ...manifest.icons,
    { src: "/apple-touch-icon.png", sizes: "180x180" },
  ]) {
    const icon = await page.request.get(src);
    expect(icon.status(), src).toBe(200);
    expect(icon.headers()["content-type"], src).toContain("image/png");
    // The size it claims is the size it is: a PNG's IHDR carries both.
    const bytes = await icon.body();
    expect(`${bytes.readUInt32BE(16)}x${bytes.readUInt32BE(20)}`, src).toBe(
      sizes,
    );
  }
});
