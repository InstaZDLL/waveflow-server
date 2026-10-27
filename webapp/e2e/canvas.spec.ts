// A track's canvas: placed, replaced, removed, and played over its cover.

import { expect, test } from "@playwright/test";

import { expectNoWcagViolations } from "./fixtures/accessibility";
import { resetAndMockApi } from "./fixtures/api";
import { library } from "./fixtures/catalogue";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

/**
 * RFC-009 from the editor: a loop sent as the file's own bytes, read back
 * through its ticket rather than assumed from the answer, refused in words,
 * and taken away.
 */
test("places, replaces and removes a track's canvas", async ({ page }) => {
  mock.libraries = [
    { ...library("library-1", "Ma musique"), accepts_canvas: true },
  ];
  await page.goto("/tracks/song-9/edit");
  await expect(page.getByText("This track has no canvas.")).toBeVisible();

  const loop = Buffer.from([0x1a, 0x45, 0xdf, 0xa3, 0x01, 0x02]);
  await page.getByLabel("Choose a loop").setInputFiles({
    name: "loop.webm",
    mimeType: "video/webm",
    buffer: loop,
  });
  await expect(page.getByText("Canvas saved.")).toBeVisible();
  const preview = page.getByLabel("Canvas preview");
  await expect(preview).toHaveAttribute(
    "src",
    "/api/v2/canvas-stream/ticket-song-9",
  );

  // Refused: said in words, and the loop that was there is still there.
  mock.refuseCanvasWith = 422;
  await page.getByLabel("Replace the loop").setInputFiles({
    name: "episode.mp4",
    mimeType: "video/mp4",
    buffer: Buffer.from([9, 9, 9]),
  });
  await expect(
    page.getByText(
      "The server refused this file: it takes a short mp4 or webm loop with a picture.",
    ),
  ).toBeVisible();
  await expect(preview).toHaveAttribute(
    "src",
    "/api/v2/canvas-stream/ticket-song-9",
  );

  await expectNoWcagViolations(page);

  await page.getByRole("button", { name: "Remove the canvas" }).click();
  await expect(page.getByText("Canvas removed.")).toBeVisible();
  await expect(page.getByText("This track has no canvas.")).toBeVisible();
  expect(mock.canvasWrites).toEqual([
    { method: "PUT", track: "song-9", bytes: [...loop] },
    { method: "PUT", track: "song-9", bytes: [9, 9, 9] },
    { method: "DELETE", track: "song-9" },
  ]);
});

/**
 * A library its operator has not opened to loops offers no new canvas, and
 * still lets go of the one a track carries: the server does not gate removal on
 * the flag, and the screen must not strand what the server would release.
 */
test("offers removal but no new canvas where the library takes none", async ({
  page,
}) => {
  mock.canvases.set("song-9", [1, 2, 3]);
  await page.goto("/tracks/song-9/edit");
  // The notice waits for the list of libraries. Until it answers the picker is
  // offered, so its absence below is only an answer once this is on screen.
  await expect(
    page.getByText(
      "This library does not take canvases. Its operator decides that on the server.",
    ),
  ).toBeVisible();
  await expect(page.getByLabel("Canvas preview")).toBeAttached();
  await expect(
    page.getByRole("button", { name: "Remove the canvas" }),
  ).toBeVisible();
  await expect(page.locator(".canvas-panel input[type=file]")).toHaveCount(0);
});

/**
 * Most tracks carry no loop, and minting a ticket is how the client asks — so
 * most of the time the answer is 404. The playing screen asks from an effect on
 * mount, which made that 404 a per-navigation cost with no end to it: measured
 * in a real session as a burst in the console, one request per visit, for ever.
 *
 * Client-side navigation both ways, deliberately. `page.goto` loads a fresh
 * document and empties everything the module remembers, so a test built on it
 * would prove the reload rather than the fix.
 */
test("asks once whether a track carries a canvas, not once per visit", async ({
  page,
}) => {
  // `canvases` is left empty, so every ticket answers 404 — the shape at issue.
  const tickets: string[] = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (path.endsWith("/canvas-ticket")) tickets.push(path);
  });

  await page.goto("/playing");
  await expect(page.locator(".canvas-stage .cover")).toBeVisible();
  await expect
    .poll(() => tickets)
    .toEqual(["/api/v2/tracks/song-1/canvas-ticket"]);

  for (const _visit of [1, 2]) {
    await page.getByRole("link", { name: "Albums" }).click();
    await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
    await page.getByRole("link", { name: "Now playing" }).click();
    await expect(page.locator(".canvas-stage .cover")).toBeVisible();
  }

  // Waited for rather than asserted straight away: a request issued by the
  // mount effect leaves after the frame that made the cover visible, so
  // counting at that moment would pass whether or not one was on its way.
  await page.waitForLoadState("networkidle");
  expect(tickets).toHaveLength(1);
});

/**
 * The loop over the cover, rendered as the desktop renders it: muted, looping
 * and hidden from assistive technology, with the cover underneath keeping its
 * name. It steps aside when motion is reduced, and when the browser cannot play
 * it — the cover shows then, never an empty frame.
 */
test("plays a track's canvas over its cover, and steps aside for it", async ({
  page,
}) => {
  mock.canvases.set("song-1", [1, 2, 3]);
  let release = () => {};
  mock.canvasStreamGate = new Promise<void>((resolve) => {
    release = () => resolve();
  });
  await page.goto("/playing");
  const stage = page.locator(".canvas-stage");
  const video = stage.locator("video");
  await expect(video).toHaveAttribute(
    "src",
    "/api/v2/canvas-stream/ticket-song-1",
  );
  await expect(video).toHaveAttribute("aria-hidden", "true");
  await expect(video).toHaveJSProperty("muted", true);
  await expect(video).toHaveJSProperty("loop", true);
  await expect(stage.locator(".cover")).toBeVisible();
  await expectNoWcagViolations(page);

  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(video).toHaveCount(0);
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await expect(video).toHaveCount(1);

  // Bytes that are not a video: the browser gives up on them, and the cover is
  // what is left rather than a frame with nothing in it.
  release();
  await expect(video).toHaveCount(0);
  await expect(stage.locator(".cover")).toBeVisible();
});

/**
 * A loop already on screen survives a failed re-mint.
 *
 * A query keeps the answer it has when a later one fails, so `data` and `error`
 * can both be set at once — and saving a correction makes that reachable, since
 * it invalidates everything and this ticket is re-minted. Reading the error
 * first replaced a preview that still played, for a credential good for an
 * hour.
 */
test("keeps a canvas on screen when re-minting its ticket fails", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "one viewport is enough");
  mock.canvases.set("song-9", [1, 2, 3]);

  await page.goto("/tracks/song-9/edit");
  const preview = page.locator("video.canvas-preview");
  await expect(preview).toBeVisible();

  // From here the ticket route is down, while the loop already on screen is
  // still playable.
  await page.route("**/api/v2/tracks/song-9/canvas-ticket", async (route) => {
    await route.fulfill({ status: 503, body: "" });
  });

  const title = page.getByLabel("Title", { exact: true });
  await title.fill("A title this track did not have");
  // Armed before the click, and awaited after it: the assertions below are
  // about what the panel shows once the re-mint has actually failed, and
  // waiting only on "Saved." would leave that to the order two promises happen
  // to settle in.
  const refused = page.waitForResponse(
    (response) =>
      response.url().endsWith("/canvas-ticket") && response.status() === 503,
  );
  await page.getByRole("button", { name: "Save" }).click();
  await refused;
  await expect(page.getByText("Saved.")).toBeVisible();

  await expect(preview).toBeVisible();
  await expect(
    page.getByText("Whether this track has a canvas could not be read."),
  ).toHaveCount(0);
});
