// Radio: the list of stations, and a live station in the player bar.

import { expect, test } from "@playwright/test";

import {
  chooseLightTheme,
  expectNoWcagViolations,
} from "./fixtures/accessibility";
import { resetAndMockApi } from "./fixtures/api";
import { PIXEL, silentWav } from "./fixtures/media";
import {
  elements,
  navigateWithin,
  NOVA,
  serveStationAndQueue,
} from "./fixtures/radio";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

/**
 * The radio page, which manages a list the catalogue knows nothing about.
 *
 * Worth its own test for one reason: a station is written and then read back,
 * so every control here depends on the list the server holds rather than on
 * what the page remembers. The harness keeps the list for that reason too — a
 * mock answering a constant would let a delete that deletes nothing pass.
 */
test("keeps a list of radio stations, replacing rather than patching", async ({
  page,
}) => {
  await page.goto("/radio");
  await expect(
    page.getByRole("heading", { name: "Radio", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("No stations yet. Add one to keep it here."),
  ).toBeVisible();

  // The form lives in a dialog the page's own button opens.
  await page.getByRole("button", { name: "Add station" }).click();
  const form = page.getByRole("dialog", { name: "Add a station" });
  await form.getByLabel("Station name").fill("Radio Nova");
  await form.getByLabel("Stream address").fill("https://nova.example.org/live");
  await form.getByLabel("Station page (optional)").fill("https://nova.example.org/");
  await form.getByRole("button", { name: "Save station" }).click();
  await expect(form).toHaveCount(0);

  const listed = page.locator(".station-grid li");
  await expect(listed).toHaveCount(1);
  await expect(listed.first()).toContainText("Radio Nova");
  await expect(listed.first()).toContainText("https://nova.example.org/live");
  await expect(
    page.getByRole("link", { name: "Station page" }),
  ).toHaveAttribute("href", "https://nova.example.org/");

  // An address the server would refuse is refused here first, with a reason,
  // and the button stays shut rather than sending a request to find out.
  await page.getByRole("button", { name: "Add station" }).click();
  await form.getByLabel("Station name").fill("Elsewhere");
  await form.getByLabel("Stream address").fill("mms://elsewhere.example.org/s");
  await expect(
    form.getByText("An address must start with http:// or https://."),
  ).toBeVisible();
  await expect(
    form.getByRole("button", { name: "Save station" }),
  ).toBeDisabled();

  await expectNoWcagViolations(page);
  await form.getByRole("button", { name: "Cancel" }).click();
  await expect(form).toHaveCount(0);

  // Editing, from the card's menu, loads the station into the same form, and
  // says what saving means: the page address is cleared because the form
  // sends every field.
  await listed.first().getByRole("button", { name: "Actions for Radio Nova" }).click();
  await page.getByRole("menuitem", { name: "Edit" }).click();
  const editor = page.getByRole("dialog", { name: "Edit “Radio Nova”" });
  await expect(editor.getByLabel("Station name")).toHaveValue("Radio Nova");
  await expect(editor.getByLabel("Station page (optional)")).toHaveValue(
    "https://nova.example.org/",
  );
  await expect(
    editor.getByText(
      "Saving replaces every field, so an empty page address clears the stored one.",
    ),
  ).toBeVisible();
  await editor.getByLabel("Station name").fill("Radio Nova Classics");
  await editor.getByLabel("Station page (optional)").fill("");
  await editor.getByRole("button", { name: "Save station" }).click();

  await expect(listed.first()).toContainText("Radio Nova Classics");
  // Cleared, which is the half a patch would have got wrong.
  await expect(page.getByRole("link", { name: "Station page" })).toHaveCount(0);

  // A delete is confirmed first, then goes.
  page.once("dialog", (question) => void question.accept());
  await listed
    .first()
    .getByRole("button", { name: "Actions for Radio Nova Classics" })
    .click();
  await page.getByRole("menuitem", { name: "Delete" }).click();
  await expect(listed).toHaveCount(0);
  await expect(
    page.getByText("No stations yet. Add one to keep it here."),
  ).toBeVisible();
});

/**
 * The form is closed for the length of a write, and the station playing is
 * not.
 *
 * **The held request below is what makes this test able to fail.** Every other
 * mock under `fixtures/` answers instantly, and against an instant answer a
 * guarded form and an unguarded one look identical: the save is over before a
 * second click or a second edit could begin. A request held open until this
 * test lets it go leaves the window open for as long as the assertions need —
 * where a fixed delay would be a race the slowest runner loses (a review
 * finding, taken; the two credential tests in `settings.spec.ts`, "shows the
 * clients that can read the library, and revokes one" and "sends one rotation
 * however many times the button is pressed", hold their requests the same
 * way).
 */
test("closes the form while a station is saving, and leaves listening alone", async ({
  page,
}) => {
  await page.goto("/radio");
  const form = page.getByRole("dialog", { name: "Add a station" });
  await page.getByRole("button", { name: "Add station" }).click();
  await form.getByLabel("Station name").fill("Radio Nova");
  await form.getByLabel("Stream address").fill("https://nova.example.org/live");
  await form.getByRole("button", { name: "Save station" }).click();
  await expect(page.locator(".station-grid li")).toHaveCount(1);
  expect(mock.radioWrites).toEqual(["POST"]);

  // Held from here on, so only the second save waits.
  let release = () => {};
  mock.radioGate = new Promise<void>((resolve) => {
    release = resolve;
  });

  await page.getByRole("button", { name: "Add station" }).click();
  await form.getByLabel("Station name").fill("FIP");
  await form.getByLabel("Stream address").fill("https://fip.example.org/live");
  await form.getByRole("button", { name: "Save station" }).click();

  // Shut for the whole round trip: the button cannot be pressed again, and the
  // fields cannot be refilled under a save that is about to clear them.
  //
  // **Holding the request is what lets the first line below distinguish**, and
  // it did not when the mock merely answered slowly: an empty form disables
  // that button too, so the assertion retried until the save landed, cleared
  // the fields and disabled the button for the other reason. Held open, the
  // save cannot land, and removing the guard fails on this very line —
  // measured both ways.
  await expect(form.getByRole("button", { name: "Save station" })).toBeDisabled();
  await expect(form.getByLabel("Station name")).toBeDisabled();
  await expect(form.getByLabel("Stream address")).toBeDisabled();
  await expect(form.getByRole("button", { name: "Cancel" })).toBeDisabled();
  // And no edit or delete can begin, which is what made a save clear a form
  // somebody else had just filled: the cards' menus are shut as well.
  await expect(
    page.locator(".station-grid li").first().locator(".action-menu-button"),
  ).toBeDisabled();
  // Listening is the deliberate exception: it writes nothing, and a reader
  // waiting on a save has no reason to lose the station they are hearing.
  await expect(
    page.locator(".station-grid li").first().locator(".station-play"),
  ).toBeEnabled();

  // The second click of an impatient double-click, dispatched for real rather
  // than waited for: `force` skips Playwright's own actionability check, so
  // what refuses it is the browser declining to activate a disabled button.
  // Without the guard this click sends a second POST and the account ends up
  // with two identical stations from one gesture — which is what the count
  // below would then show.
  await form
    .getByRole("button", { name: "Save station" })
    .click({ force: true });

  release();

  await expect(page.locator(".station-grid li")).toHaveCount(2);
  // One gesture, one station. Two POSTs in all, one per click.
  expect(mock.radioWrites).toEqual(["POST", "POST"]);
  // Closed once the save has landed, and empty when it is opened again.
  await expect(form).toHaveCount(0);
  await page.getByRole("button", { name: "Add station" }).click();
  await expect(form.getByLabel("Station name")).toBeEnabled();
  await expect(form.getByLabel("Station name")).toHaveValue("");
});

/**
 * A station plays in the player bar, not on the radio page: the page used to
 * own a private element and pause it on the way out, because it held the only
 * stop button there was. Now the bar is that button, and the station carries
 * on wherever the listener goes.
 *
 * **And it never goes through the ReplayGain graph.** A station is an address
 * on another origin, and a cross-origin stream without CORS headers routed
 * through a `MediaElementSource` plays as silence while the element reports
 * it playing. ReplayGain is on here and a track has built the graph, so the
 * graph exists and has claimed the queue's element; the station's must not be
 * that element, nor be claimed by it.
 */
test("carries a live station in the bar across pages, off the ReplayGain graph", async ({
  page,
}) => {
  await serveStationAndQueue(page);
  await page.goto("/albums/album-2");
  await page.getByRole("button", { name: "Play album" }).click();
  const bar = page.locator(".player");
  await expect(bar.locator(".player-track strong")).toHaveText("Hidden Place");
  // The graph is built, and holds the queue's element.
  await expect.poll(async () => (await elements(page)).queue?.claimed).toBe(true);

  await navigateWithin(page, "Radio");
  const station = page.locator(".station-grid li").filter({ hasText: "Radio Nova" });
  await station.getByRole("button", { name: "Listen" }).click();

  await expect(bar.locator(".player-track strong")).toHaveText("Radio Nova");
  await expect(bar.locator(".live-badge")).toHaveText("Live");
  await expect(station.getByRole("button", { name: "Stop" })).toBeVisible();
  await expect
    .poll(async () => (await elements(page)).station?.time ?? 0)
    .toBeGreaterThan(0.2);
  const playing = await elements(page);
  expect(playing.station).toMatchObject({ paused: false, source: NOVA, claimed: false });
  // Its own element: the queue's is paused under it, still the graph's, and
  // still holding its track.
  expect(playing.queue).toMatchObject({ paused: true, claimed: true });
  expect(playing.claims).toBe(1);
  // A station has nothing to seek in and nothing to go to next.
  await expect(bar.getByRole("slider", { name: "Seek" })).toHaveCount(0);
  await expect(bar.getByRole("button", { name: "Next track" })).toHaveCount(0);
  await expect(bar.getByRole("button", { name: "Previous track" })).toHaveCount(0);
  await expect(bar.getByRole("button", { name: "Shuffle" })).toHaveCount(0);
  await expectNoWcagViolations(page);

  // Away from the radio page, which used to stop it: still live, still heard.
  await navigateWithin(page, "Albums");
  await expect(page.locator(".grid li").first()).toBeVisible();
  await expect(bar.locator(".player-track strong")).toHaveText("Radio Nova");
  await expect(bar.locator(".live-badge")).toHaveText("Live");  const before = (await elements(page)).station?.time ?? 0;
  await expect
    .poll(async () => (await elements(page)).station?.time ?? 0)
    .toBeGreaterThan(before);
  expect((await elements(page)).station?.paused).toBe(false);

  // The bar is the stop button now.
  await bar.getByRole("button", { name: "Stop the station" }).click();
  await expect.poll(async () => (await elements(page)).station?.paused).toBe(true);
  await expect(bar.getByRole("button", { name: "Play" })).toBeVisible();
  await expect(bar.locator(".player-track strong")).toHaveText("Radio Nova");
});

/**
 * Starting a station pauses the music and keeps the queue; playing a track
 * afterwards takes the speakers back. One audible source, always.
 */
test("pauses the queue under a station, keeps it, and gives it back to a track", async ({
  page,
}) => {
  await serveStationAndQueue(page);
  await page.goto("/albums/album-2");
  await page.getByRole("button", { name: "Play album" }).click();
  const bar = page.locator(".player");
  await expect(bar.getByRole("button", { name: "Pause" })).toBeVisible();

  await navigateWithin(page, "Radio");
  await page.getByRole("button", { name: "Listen" }).click();
  await expect(bar.locator(".live-badge")).toHaveText("Live");
  await expect.poll(async () => (await elements(page)).station?.paused).toBe(false);
  expect((await elements(page)).queue?.paused).toBe(true);

  // Paused, not cleared: the queue is all there.
  await navigateWithin(page, "Queue");
  const rows = page.locator(".queue-list li");
  await expect(rows).toHaveCount(3);
  await expect(rows.first()).toHaveClass("active");
  expect((await elements(page)).station?.paused).toBe(false);

  // A track takes over from the station, in the bar and in the speakers.
  await rows.filter({ hasText: "Cocoon" }).locator(".queue-track").click();
  await expect(bar.locator(".player-track strong")).toHaveText("Cocoon");
  await expect(bar.locator(".live-badge")).toHaveCount(0);
  await expect.poll(async () => (await elements(page)).station?.paused).toBe(true);
  await expect
    .poll(async () => (await elements(page)).queue)
    .toMatchObject({ paused: false, source: expect.stringContaining("ticket-song-2") });
  // And the radio page no longer calls the station playing.
  await navigateWithin(page, "Radio");
  await expect(page.getByRole("button", { name: "Listen" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Stop" })).toHaveCount(0);
});

/**
 * A station's image is an address the account types, sent with the station;
 * the server fetches it once and answers a hash, and what the page shows is
 * the artwork route's answer for that hash — never the image's own host,
 * which RFC-019's `img-src` would refuse and which would see the listener.
 */
test("takes an image address for a station, and shows the image the server kept", async ({
  page,
}) => {
  const asked: string[] = [];
  page.on("request", (request) => {
    if (new URL(request.url()).hostname === "logos.example.org") {
      asked.push(request.url());
    }
  });
  await page.route("**/api/v2/artwork/station-art-*", (route) =>
    route.fulfill({ contentType: "image/png", body: PIXEL }),
  );
  await page.goto("/radio");
  await page.getByRole("button", { name: "Add station" }).click();
  const form = page.getByRole("dialog", { name: "Add a station" });
  await form.getByLabel("Station name").fill("Radio Nova");
  await form.getByLabel("Stream address").fill(NOVA);
  // Refused here first, as the server would, and the button stays shut.
  await form.getByLabel("Station image (optional)").fill("ftp://logos.example.org/n.png");
  await expect(
    form.getByText("An address must start with http:// or https://."),
  ).toBeVisible();
  await expect(form.getByRole("button", { name: "Save station" })).toBeDisabled();
  await form
    .getByLabel("Station image (optional)")
    .fill("https://logos.example.org/nova.png");
  await expect(
    form.getByText("The server downloads it once, when you save", { exact: false }),
  ).toBeVisible();
  await form.getByRole("button", { name: "Save station" }).click();

  const listed = page.locator(".station-grid li").filter({ hasText: "Radio Nova" });
  await expect(listed.locator("img.station-cover")).toBeVisible();
  expect(mock.stationImages).toEqual(["https://logos.example.org/nova.png"]);

  // Edited, the form shows the address back and sends it again; cleared, it
  // sends `null` and the station keeps the symbol.
  await listed.getByRole("button", { name: "Actions for Radio Nova" }).click();
  await page.getByRole("menuitem", { name: "Edit" }).click();
  const editor = page.getByRole("dialog", { name: "Edit “Radio Nova”" });
  await expect(editor.getByLabel("Station image (optional)")).toHaveValue(
    "https://logos.example.org/nova.png",
  );
  await editor.getByLabel("Station image (optional)").fill("");
  await editor.getByRole("button", { name: "Save station" }).click();
  await expect(listed.locator(".station-cover.cover-fallback svg")).toBeVisible();
  expect(mock.stationImages).toEqual(["https://logos.example.org/nova.png", null]);
  await expectNoWcagViolations(page);
  expect(asked, "the browser never asks the image's own host").toEqual([]);
});

for (const theme of ["dark", "light"] as const) {
  test(`shows a station's image in the bar, and the symbol without one, in the ${theme}`, async ({
    page,
  }) => {
    if (theme === "light") await chooseLightTheme(page);
    await serveStationAndQueue(page);
    mock.stations = [
      { ...mock.stations[0], image_url: "https://logos.example.org/nova.png", artwork_hash: "station-art-nova" },
      {
        ...mock.stations[0],
        id: "station-2",
        name: "Radio Plain",
      },
    ];
    let served = 0;
    await page.route("**/api/v2/artwork/station-art-nova", (route) => {
      served += 1;
      return route.fulfill({ contentType: "image/png", body: PIXEL });
    });
    await page.goto("/radio");
    const nova = page.locator(".station-grid li").filter({ hasText: "Radio Nova" });
    await expect(nova.locator("img.station-cover")).toBeVisible();
    await nova.getByRole("button", { name: "Listen" }).click();

    const bar = page.locator(".player");
    await expect(bar.locator(".player-track strong")).toHaveText("Radio Nova");
    await expect(bar.locator("img.player-cover")).toBeVisible();
    // The system's controls are given the same cover, by its object URL.
    await expect
      .poll(() =>
        page.evaluate(
          () => navigator.mediaSession?.metadata?.artwork?.[0]?.src ?? "",
        ),
      )
      .toMatch(/^blob:/);
    expect(served).toBe(1);
    await expectNoWcagViolations(page);

    // A station without an image keeps the radio symbol.
    const plain = page.locator(".station-grid li").filter({ hasText: "Radio Plain" });
    await plain.getByRole("button", { name: "Listen" }).click();
    await expect(bar.locator(".player-track strong")).toHaveText("Radio Plain");
    await expect(bar.locator(".player-cover.cover-fallback svg")).toBeVisible();
    await expect(bar.locator("img.player-cover")).toHaveCount(0);
    await expect
      .poll(() =>
        page.evaluate(() => navigator.mediaSession?.metadata?.artwork?.length ?? 0),
      )
      .toBe(0);
  });
}

/**
 * Taking a station out of the bar leaves the queue's track there, paused
 * where it was, and the light theme keeps the live badge readable.
 */
test("keeps a live station readable in the light, and closes back to the queue", async ({
  page,
}) => {
  await chooseLightTheme(page);
  await serveStationAndQueue(page);
  await page.goto("/radio");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "default");
  await page.getByRole("button", { name: "Listen" }).click();
  const bar = page.locator(".player");
  await expect(bar.locator(".live-badge")).toHaveText("Live");
  await expect.poll(async () => (await elements(page)).station?.paused).toBe(false);
  await expectNoWcagViolations(page);
  await bar.getByRole("button", { name: "Stop and close" }).click();
  await expect.poll(async () => (await elements(page)).station?.paused).toBe(true);
  // The saved queue's track, still where the listener left it.
  await expect(bar.locator(".player-track strong")).toHaveText("Hidden Place");
  await expect(bar.getByRole("button", { name: "Play" })).toBeVisible();
});

/**
 * A station that failed can be tried again, from its page or from the bar.
 *
 * A source that fails leaves the element's `paused` at false — the failure
 * steps reject the play, they do not pause — so a player that took `paused`
 * for the truth thought the failed station was playing: Listen did nothing,
 * and the bar's play button stopped it instead of starting it. The station
 * answers 404 once, then plays.
 */
for (const from of ["page", "bar"] as const) {
  test(`tries a failed station again from the ${from}`, async ({ page }) => {
    await serveStationAndQueue(page);
    let asked = 0;
    const audio = silentWav(60);
    await page.route(`${NOVA}**`, (route) => {
      asked += 1;
      return asked === 1
        ? route.fulfill({ status: 404, body: "" })
        : route.fulfill({ body: audio, headers: { "content-type": "audio/wav" } });
    });
    await page.goto("/radio");
    await page.getByRole("button", { name: "Listen" }).click();
    await expect(
      page.getByText("That station could not be played.", { exact: false }),
    ).toBeVisible();
    const bar = page.locator(".player");
    await expect(bar.getByRole("alert")).toHaveText("This station could not be played.");
    expect(asked).toBe(1);

    if (from === "page") {
      await page.getByRole("button", { name: "Listen" }).click();
    } else {
      await bar.getByRole("button", { name: "Play" }).click();
    }
    await expect.poll(() => asked).toBe(2);
    await expect
      .poll(async () => (await elements(page)).station?.time ?? 0)
      .toBeGreaterThan(0.2);
    expect((await elements(page)).station?.paused).toBe(false);
    await expect(bar.getByRole("alert")).toHaveCount(0);
    await expect(bar.getByRole("button", { name: "Stop the station" })).toBeVisible();
  });
}

/**
 * Signing out stops a station too, as it stops the music (#329, #330): the
 * station's element is the player's, and the player goes with the session.
 */
test("stops a live station when the listener signs out", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "signs out from the sidebar");
  await serveStationAndQueue(page);
  await page.goto("/radio");
  await page.getByRole("button", { name: "Listen" }).click();
  await expect.poll(async () => (await elements(page)).station?.paused).toBe(false);
  // Dispatched rather than clicked, as in the cache test: the sidebar's
  // footer sits under the fixed player bar at this viewport.
  await page.getByRole("button", { name: "Sign out" }).dispatchEvent("click");
  await expect(page).toHaveURL(/\/login$/);
  await expect.poll(async () => (await elements(page)).station?.paused).toBe(true);
});
