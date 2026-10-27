// External scrobbling (RFC-010): instances, links and ambiguous listens.

import { expect, test } from "@playwright/test";

import { expectNoWcagViolations } from "./fixtures/accessibility";
import { resetAndMockApi } from "./fixtures/api";
import { linkState } from "./fixtures/catalogue";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

/**
 * External scrobbling — RFC-010, the four gestures a client owes.
 *
 * The path is the server's: `lastfm_callback` redirects to
 * `/settings/scrobbling`, so a page anywhere else leaves the last step of the
 * journey on the not-found screen.
 */
test("offers each instance by name, and says why one cannot be linked", async ({
  page,
}) => {
  await page.goto("/settings/scrobbling");

  await expect(
    page.getByRole("heading", { name: "External scrobbling" }),
  ).toBeVisible();
  // Two instances of one recipient, each named. A screen keyed on the
  // recipient would show "Maloja" once and hide whichever it linked second.
  await expect(page.getByText("Maloja — instance “alice”")).toBeVisible();
  await expect(page.getByText("Maloja — instance “bob”")).toBeVisible();
  await expect(
    page.getByText("ListenBrainz — instance “default”"),
  ).toBeVisible();

  // Declared and unusable is a real shape, and the reason is published so the
  // person is not left to discover it when a link fails later. Asserted on the
  // words this client chose for `NO_PUBLIC_URL`, not on the code it was given:
  // a case printed raw would satisfy any assertion made on the code itself.
  await expect(
    page.getByText(
      "Unavailable here: the browser journey needs this server’s public " +
        "address to be an https one — an operator can still link an account " +
        "from the command line",
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Connect to Last.fm" }),
  ).toHaveCount(0);

  await expectNoWcagViolations(page);
});

/**
 * A server newer than the client reading it.
 *
 * `unavailable` carries a case rather than a sentence, so this client has a
 * table of words for the cases it knows — and a case it does not know must not
 * become a blank where a reason should be. The type is open for the same
 * reason: a union naming only today's cases would be an assertion about the
 * wire that the wire never made, which is the fault this whole change set
 * exists to remove.
 */
test("says a reason it does not recognise is a reason, not a blank", async ({
  page,
}) => {
  const lastfm = mock.destinations.find((row) => row.provider === "lastfm");
  if (lastfm) lastfm.unavailable = "a_case_from_a_later_release";

  await page.goto("/settings/scrobbling");

  await expect(
    page.getByText("Unavailable here: this server did not say why"),
  ).toBeVisible();
});

/**
 * `toString` is not a translation key.
 *
 * Opening the union widened the lookup key to `string`, and a plain object
 * answers for names nobody put in it: `toString` and `constructor` come back
 * off the prototype, truthy, and would reach `t()` as if they were keys. Every
 * inherited name is a case this client does not know, and must read as one.
 */
for (const inherited of ["toString", "constructor", "hasOwnProperty"]) {
  test(`treats the inherited name ${inherited} as a case it does not know`, async ({
    page,
  }) => {
    const lastfm = mock.destinations.find((row) => row.provider === "lastfm");
    if (lastfm) lastfm.unavailable = inherited;

    await page.goto("/settings/scrobbling");

    await expect(
      page.getByText("Unavailable here: this server did not say why"),
    ).toBeVisible();
  });
}

test("links one instance by its key, and leaves the other alone", async ({
  page,
}) => {
  await page.goto("/settings/scrobbling");
  await expect(page.getByText("Maloja — instance “alice”")).toBeVisible();

  await page
    .getByLabel("API key: Maloja — instance “alice”")
    .fill("alice-api-key");
  await page
    .locator(".scrobble-row", { hasText: "alice" })
    .getByRole("button", { name: "Link" })
    .click();

  await expect(
    page.getByRole("button", { name: "Unlink: Maloja — instance “alice”" }),
  ).toBeVisible();
  expect(mock.scrobbleWrites).toEqual([
    {
      method: "PUT",
      path: "/api/v2/scrobble-links/maloja/alice",
      body: { secret: "alice-api-key" },
    },
  ]);
  // The pair names the resource: bob is untouched and still offers its field.
  await expect(
    page.getByLabel("API key: Maloja — instance “bob”"),
  ).toBeVisible();

  // The queue behind the link is a state and four counters, never an echo of
  // what was heard.
  await expect(page.getByText("The queue is not moving")).toBeVisible();
  await expect(page.getByText("12 waiting")).toBeVisible();
  await expect(page.getByText("1 waiting on you")).toBeVisible();
  await expect(
    page.getByText("Last refusal: the destination asked for room."),
  ).toBeVisible();
});

test("sends the browser to Last.fm rather than asking for a key", async ({
  page,
}) => {
  // Last.fm hands out no secret a person can hold, so the only gesture is a
  // journey. Declared *and* usable here, unlike the listing test.
  mock.destinations = [
    { provider: "lastfm" as const, destination: "default", available: true },
  ];
  await page.route("https://www.last.fm/**", (route) =>
    route.fulfill({ status: 200, body: "<title>last.fm</title>" }),
  );

  await page.goto("/settings/scrobbling");
  await page.getByRole("button", { name: "Connect to Last.fm" }).click();

  await page.waitForURL("https://www.last.fm/**");
  expect(mock.scrobbleWrites).toEqual([
    {
      method: "POST",
      path: "/api/v2/scrobble-links/lastfm/authorize/default",
      body: undefined,
    },
  ]);
});

test("says so when the journey comes back, and names the instance", async ({
  page,
}) => {
  // What the callback redirects to once it has finished the link itself. The
  // value is shown and nothing more: anybody can type this address, and the
  // listing beside it is what actually says a link exists.
  await page.goto(
    "/settings/scrobbling?linked=lastfm&destination=default&keep=me",
  );

  await expect(page.getByText("Last.fm is linked: default.")).toBeVisible();

  // Said once, then spent. A reload is a later navigation and not this one, so
  // the pair leaves the address while the notice stays on the page — and a URL
  // copied out of the bar no longer tells somebody else their Last.fm is
  // linked. Anything else in the query is left where it was.
  await expect(page).toHaveURL(/\/settings\/scrobbling\?keep=me$/);
  await page.reload();
  await expect(page.getByText("Last.fm is linked: default.")).toHaveCount(0);
});

test("names the track behind an ambiguous listen, and answers it once", async ({
  page,
}) => {
  mock.scrobbleLinks = [linkState("maloja", "alice")];
  await page.goto("/settings/scrobbling");

  const entry = page.locator(".scrobble-uncertain li");
  // The entry carries no title — decision 12 keeps the envelope out of the API
  // — so the screen matches `played_at` against the account's own history,
  // which is what that field is documented to be for. A bare timestamp would
  // ask somebody to decide about a listen they cannot recognise.
  await expect(entry.getByText("Track t5")).toBeVisible();
  await expect(entry.getByText(/instance “alice”/)).toBeVisible();
  await expect(entry.getByText(/the answer never arrived/)).toBeVisible();

  await entry.getByRole("button", { name: "Send it again" }).click();

  await expect(page.getByText("Nothing is waiting on you.")).toBeVisible();
  expect(mock.scrobbleWrites).toEqual([
    { method: "POST", path: "/api/v2/scrobble-queue/uncertain/entry-1/retry" },
  ]);
});

test("throws an ambiguous listen away when that is the answer", async ({
  page,
}) => {
  mock.scrobbleLinks = [linkState("maloja", "alice")];
  await page.goto("/settings/scrobbling");

  const entry = page.locator(".scrobble-uncertain li");
  await expect(entry.getByText("Track t5")).toBeVisible();
  await entry.getByRole("button", { name: "Throw it away" }).click();

  await expect(page.getByText("Nothing is waiting on you.")).toBeVisible();
  expect(mock.scrobbleWrites).toEqual([
    { method: "DELETE", path: "/api/v2/scrobble-queue/uncertain/entry-1" },
  ]);
});

test("unlinks one instance and leaves a namesake at another recipient", async ({
  page,
}) => {
  // `default` is the name every recipient uses, so the pair is the only thing
  // that tells these two apart. A screen — or a mock — matching on the
  // instance name alone withdraws both, which is the fault the unique index
  // on `(user_id, provider, destination)` was rebuilt to make impossible.
  mock.destinations = [
    {
      provider: "listenbrainz" as const,
      destination: "default",
      available: true,
    },
    { provider: "maloja" as const, destination: "default", available: true },
  ];
  mock.scrobbleLinks = [
    linkState("listenbrainz", "default"),
    linkState("maloja", "default"),
  ];

  await page.goto("/settings/scrobbling");
  await page
    .getByRole("button", {
      name: "Unlink: ListenBrainz — instance “default”",
    })
    .click();

  await expect(
    page.getByRole("button", { name: "Unlink: Maloja — instance “default”" }),
  ).toBeVisible();
  await expect(
    page.getByLabel("API key: ListenBrainz — instance “default”"),
  ).toBeVisible();
  expect(mock.scrobbleWrites).toEqual([
    { method: "DELETE", path: "/api/v2/scrobble-links/listenbrainz/default" },
  ]);
});
