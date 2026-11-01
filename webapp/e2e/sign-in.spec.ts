// The sign-in screen, and the first-run setup it doubles as.

import { expect, type Request, test } from "@playwright/test";

import { resetAndMockApi } from "./fixtures/api";
import { session } from "./fixtures/catalogue";

test.beforeEach(resetAndMockApi);

/**
 * A server in laptop mode opens the browser on `/login#setup-token=…` on its
 * first start (`src/app/laptop/browser.rs`). The fragment is a hand-over: the
 * token fills the field, leaves the address bar, and reaches the server only
 * in the body of the setup request it was handed over for — never in a URL,
 * where a log, a proxy or a `Referer` would keep it.
 */
test("a setup token handed over in the fragment fills the field and travels in no URL", async ({
  page,
}) => {
  const token = "wfsetup_e2e-handed-over_token";
  const requests: Request[] = [];
  page.on("request", (request) => requests.push(request));
  let setupBody: unknown = null;
  await page.route("**/api/v2/setup", async (route) => {
    if (route.request().method() === "POST") {
      setupBody = route.request().postDataJSON();
      await route.fulfill({ status: 201, json: {} });
      return;
    }
    await route.fulfill({ json: { required: true } });
  });
  await page.route("**/api/v2/web/auth/login", (route) =>
    route.fulfill({ json: session }),
  );

  await page.goto(`/login#setup-token=${token}`);

  await expect(
    page.getByRole("heading", { name: "Create the administrator" }),
  ).toBeVisible();
  await expect(page.getByLabel("Setup token")).toHaveValue(token);
  await expect.poll(() => new URL(page.url()).hash).toBe("");
  expect(new URL(page.url()).pathname).toBe("/login");

  await page.getByLabel("Username").fill("nocturne");
  await page.getByLabel("Password").fill("correct horse battery staple");
  await page.getByRole("button", { name: "Create and sign in" }).click();
  await expect.poll(() => setupBody).not.toBeNull();
  expect(setupBody).toMatchObject({ setup_token: token });

  // Every request the page made. Not the navigation that opened it: the
  // browser keeps the fragment off the wire, but Playwright reports that
  // request's URL with the fragment it was given.
  const made = requests.filter((request) => !request.isNavigationRequest());
  expect(made.length).toBeGreaterThan(0);
  for (const request of made) {
    expect(request.url()).not.toContain(token);
    const headers = await request.allHeaders();
    for (const value of Object.values(headers)) {
      expect(value).not.toContain(token);
    }
    // The one body allowed to carry it is the one it was handed over for.
    if (!new URL(request.url()).pathname.endsWith("/api/v2/setup")) {
      expect(request.postData() ?? "").not.toContain(token);
    }
  }
});
