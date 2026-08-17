import AxeBuilder from "@axe-core/playwright";
import { expect, type Page, test } from "@playwright/test";

const session = {
  access_token: "e2e-access-token",
  user: { id: "user-1", username: "listener", role: "admin" },
  device_id: "device-1",
};

const albums = [
  {
    id: "album-1",
    library_id: "library-1",
    title: "Post",
    artist: "Björk",
    artist_id: "artist-1",
    artwork_hash: null,
    year: 1995,
    starred_at: null,
    user_rating: null,
  },
  {
    id: "album-2",
    library_id: "library-1",
    title: "Vespertine",
    artist: "Björk",
    artist_id: "artist-1",
    artwork_hash: null,
    year: 2001,
    starred_at: null,
    user_rating: null,
  },
];

async function mockAuthenticatedApi(page: Page) {
  await page.context().addCookies([
    {
      name: "waveflow-csrf",
      value: "e2e-csrf",
      url: "http://127.0.0.1:4173",
    },
  ]);
  await page.route("**/api/v2/**", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/api/v2/web/auth/refresh") {
      await route.fulfill({ json: session });
      return;
    }
    if (url.pathname === "/api/v2/albums") {
      await route.fulfill({ json: albums });
      return;
    }
    await route.fulfill({ status: 404, json: { error: "not found" } });
  });
}

test.beforeEach(async ({ page }) => {
  await mockAuthenticatedApi(page);
});

test("renders the studio shell and persists appearance preferences", async ({
  page,
}, testInfo) => {
  await page.goto("/");

  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(page.getByText("Post", { exact: true })).toBeVisible();
  await expect(page.getByText("Vespertine", { exact: true })).toBeVisible();

  const preferences =
    testInfo.project.name === "mobile"
      ? page.locator(".mobile-header")
      : page.locator(".sidebar");
  const theme = preferences.getByLabel("Theme");
  await theme.selectOption({ index: 1 });
  const selectedTheme = await theme.inputValue();
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("waveflow.theme")))
    .toBe(selectedTheme);

  const language = preferences.getByLabel("Language");
  await language.selectOption("fr");
  await expect(page.getByRole("link", { name: "Recherche" }).first()).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("lang", "fr");

  if (testInfo.project.name === "mobile") {
    await expect(page.locator(".mobile-navigation")).toBeVisible();
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(1);
  } else {
    await expect(page.locator(".sidebar")).toBeVisible();
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
  await expect(page.getByRole("link", { name: "Back to the library" })).toBeVisible();
});

test("has no automated WCAG A or AA violations", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  expect(results.violations).toEqual([]);
});
