// A track's menu, its properties, its download and its tag corrections.

import { expect, test } from "@playwright/test";

import {
  chooseLightTheme,
  expectNoWcagViolations,
} from "./fixtures/accessibility";
import { resetAndMockApi } from "./fixtures/api";
import { albumDetail, library } from "./fixtures/catalogue";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

/**
 * The right click on a track, and the properties it opens (RFC-012 decision 4).
 *
 * Driven from the keyboard first, because a menu reachable only by a right
 * click is one the accessibility gate is right to fail: Shift+F10 on the
 * focused row, arrows to Properties, Enter. The panel shows where the file
 * sits in its library — the one field only `GET /tracks/{id}` sends — and a
 * corrected tag beside what the file said. Escape gives focus back to the row
 * the menu was opened from, through the menu that no longer exists.
 */
test("opens a track's properties from its context menu", async ({ page }) => {
  await page.route("**/api/v2/songs?*", (route) =>
    route.fulfill({ json: [mock.correctable.song] }),
  );
  await page.route("**/api/v2/tracks/song-9", (route) =>
    route.fulfill({
      json: {
        ...mock.correctable.song,
        relative_path: "Björk/Post/09 Army Of Me.flac",
        full_hash: "a".repeat(64),
        created_at: Date.UTC(2026, 8, 15, 12, 0),
        album_artist: "Björk",
        comment: "Remastered\nBonus edition",
        musicbrainz_id: null,
        isrc: ["GBAAA9500009"],
        bpm: 92,
        replay_gain_track_gain: -6.5,
        replay_gain_track_peak: 0.98,
        replay_gain_album_gain: null,
        replay_gain_album_peak: null,
      },
    }),
  );
  await page.goto("/songs");
  const row = page.locator(".songs tbody tr").first();
  const title = row.locator("button.song-title");
  await title.focus();

  await page.keyboard.press("Shift+F10");
  const menu = page.getByRole("menu", { name: "Actions for Army Of Me" });
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("menuitem")).toHaveText([
    "Play",
    "Add to queue",
    "Download",
    // RFC-022: this browser supports Cache Storage, so it offers the copy.
    "Keep for offline",
    // Where the track belongs, reachable from any list and on a phone.
    "Go to album",
    "Go to Skunk Anansie",
    "Properties",
    "Edit tags",
    "Move to folder…",
  ]);
  await expect(menu.getByRole("menuitem", { name: "Play" })).toBeFocused();
  // Where focus is shows on its own, not only as a faint tint.
  await expect(menu.getByRole("menuitem", { name: "Play" })).toHaveCSS(
    "outline-style",
    "solid",
  );
  // Past "Keep for offline" (RFC-022) and the album and artist entries, the
  // seventh item.
  for (let press = 0; press < 6; press += 1) {
    await page.keyboard.press("ArrowDown");
  }
  await expect(
    menu.getByRole("menuitem", { name: "Properties" }),
  ).toBeFocused();
  await page.keyboard.press("Enter");

  const panel = page.getByRole("dialog", { name: "Army Of Me" });
  await expect(panel).toBeVisible();
  await expect(menu).toHaveCount(0);
  await expect(panel).toContainText("Björk/Post/09 Army Of Me.flac");
  await expect(panel).toContainText("a".repeat(64));
  await expect(panel).toContainText("957 kbps");
  await expect(panel).toContainText("44.1 kHz");
  await expect(panel).toContainText("-6.5 dB, peak 0.98");
  // The comment is a correction; the file carried none.
  await expect(panel).toContainText("Corrected — the file says —");
  await expectNoWcagViolations(page);

  // A click in the panel's own padding lands on the dialog element, as a
  // click on the backdrop does; only the second may close it.
  await panel.click({ position: { x: 6, y: 6 } });
  await expect(panel).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(panel).toHaveCount(0);
  await expect(title).toBeFocused();

  // The backdrop, outside the panel's box, does close it.
  await page.keyboard.press("Shift+F10");
  for (let press = 0; press < 6; press += 1) {
    await page.keyboard.press("ArrowDown");
  }
  await page.keyboard.press("Enter");
  await expect(panel).toBeVisible();
  await page.mouse.click(2, 2);
  await expect(panel).toHaveCount(0);

  // The pointer reaches the same menu, and a click away closes it.
  await row.click({ button: "right" });
  await expect(menu).toBeVisible();
  await page.mouse.click(5, 5);
  await expect(menu).toHaveCount(0);

  // The wheel closes it even turned over the menu, which does not scroll:
  // the page beneath would move and leave the menu off its row. Dispatched
  // on the entry itself, because a real wheel in a headless browser brings
  // side effects of its own that close the menu either way.
  await row.click({ button: "right" });
  await expect(menu).toBeVisible();
  await menu
    .getByRole("menuitem", { name: "Play" })
    .dispatchEvent("wheel", { deltaY: 120, bubbles: true });
  await expect(menu).toHaveCount(0);
});

/**
 * The lyrics in a track's properties (RFC-012 decision 6).
 *
 * The sheets come in the server's order, the first marked as the one shown,
 * each naming its source. An owner uploads a sheet as the bytes it is and
 * takes an upload back; anyone downloads the shown sheet, saved under the
 * name the server gives in `Content-Disposition`.
 */
test("uploads, removes and downloads lyrics from a track's properties", async ({
  page,
}) => {
  const sheet = (source: string, synced: boolean, value: string) => ({
    displayArtist: "Björk",
    displayTitle: "Army Of Me",
    lang: "eng",
    synced,
    line: [{ start: synced ? 1000 : undefined, value }],
    source,
  });
  let sheets = [sheet("sidecar_lrc", true, "from the sidecar")];
  const sent: Array<{ method: string; path: string; body: string }> = [];
  await page.route("**/api/v2/songs?*", (route) =>
    route.fulfill({ json: [mock.correctable.song] }),
  );
  await page.route("**/api/v2/tracks/song-9/lyrics", (route) =>
    route.fulfill({
      json: { trackId: "song-9", structuredLyrics: sheets },
    }),
  );
  await page.route("**/api/v2/tracks/song-9/lyrics/ttml", (route) => {
    const request = route.request();
    sent.push({
      method: request.method(),
      path: "ttml",
      body: request.postData() ?? "",
    });
    if (request.method() === "PUT") {
      sheets = [sheet("upload_ttml", true, "bonjour"), ...sheets];
      return route.fulfill({
        json: { trackId: "song-9", structuredLyrics: sheets },
      });
    }
    sheets = sheets.filter((entry) => entry.source !== "upload_ttml");
    return route.fulfill({ status: 204, body: "" });
  });
  await page.route("**/api/v2/tracks/song-9/lyrics/file", (route) =>
    route.fulfill({
      body: "[00:01.00]from the sidecar\n",
      headers: {
        "content-type": "application/octet-stream",
        "content-disposition":
          "attachment; filename=\"Bj_rk - Army Of Me.lrc\"; filename*=UTF-8''Bj%C3%B6rk%20-%20Army%20Of%20Me.lrc",
      },
    }),
  );
  await page.goto("/songs");
  await page.evaluate(() => {
    const seen = window as unknown as { __saved: string[] };
    seen.__saved = [];
    HTMLAnchorElement.prototype.click = function (this: HTMLAnchorElement) {
      seen.__saved.push(this.download);
    };
  });
  await page
    .locator(".songs tbody tr")
    .first()
    .locator("button.song-title")
    .focus();
  await page.keyboard.press("Shift+F10");
  await page
    .getByRole("menu", { name: "Actions for Army Of Me" })
    .getByRole("menuitem", { name: "Properties" })
    .click();
  const panel = page.getByRole("dialog", { name: "Army Of Me" });
  const lyrics = panel.locator(".lyrics-section");
  await expect(lyrics).toContainText("Shown");
  await expect(lyrics).toContainText("LRC file beside the track");

  // An upload goes as the file's own bytes, and leads once it is in.
  await panel.locator('input[type="file"]').setInputFiles({
    name: "Army Of Me.ttml",
    mimeType: "application/ttml+xml",
    buffer: Buffer.from('<tt xmlns="http://www.w3.org/ns/ttml"/>'),
  });
  await expect(lyrics).toContainText("Uploaded TTML");
  expect(sent).toEqual([
    {
      method: "PUT",
      path: "ttml",
      body: '<tt xmlns="http://www.w3.org/ns/ttml"/>',
    },
  ]);
  await expect(lyrics.locator(".property-row").first()).toContainText(
    "Uploaded TTML",
  );

  // A file that is not a sheet is refused before anything is sent.
  await panel.locator('input[type="file"]').setInputFiles({
    name: "notes.srt",
    mimeType: "text/plain",
    buffer: Buffer.from("1"),
  });
  await expect(lyrics.getByRole("alert")).toHaveText(
    "Choose a .lrc, .txt or .ttml file.",
  );
  expect(sent).toHaveLength(1);

  // Taking the upload back leaves the scanned sheet shown again.
  await lyrics.getByRole("button", { name: "Remove the TTML" }).click();
  await expect(lyrics).not.toContainText("Uploaded TTML");
  expect(sent.at(-1)?.method).toBe("DELETE");

  // The download is saved under the server's name, accents and all.
  await lyrics.getByRole("button", { name: "Download lyrics" }).click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as unknown as { __saved: string[] }).__saved),
    )
    .toEqual(["Björk - Army Of Me.lrc"]);
  await expectNoWcagViolations(page);
});

/**
 * Download is the stream ticket with `?download=1` (RFC-012 decision 5).
 *
 * The browser cannot send the session on a navigation, so the menu mints a
 * ticket and saves from it through a link with `download` and no value,
 * which leaves the file's name to the server's `Content-Disposition`. That
 * header is asserted server-side, against a real file; what is asserted here
 * is the link the page builds. Chromium hands a `download` link straight to
 * its download manager, past every route a test could install, so the link
 * is caught as it is clicked rather than on the wire.
 */
test("downloads a track from its context menu", async ({ page }) => {
  await page.route("**/api/v2/songs?*", (route) =>
    route.fulfill({ json: [mock.correctable.song] }),
  );
  await page.route("**/api/v2/tracks/song-9/stream-ticket", (route) =>
    route.fulfill({
      json: { url: "/api/v2/stream/ticket-9", expires_at: Date.now() + 60_000 },
    }),
  );
  await page.goto("/songs");
  await page.evaluate(() => {
    const seen = window as unknown as {
      __saved: Array<{ href: string | null; download: string }>;
    };
    seen.__saved = [];
    HTMLAnchorElement.prototype.click = function (this: HTMLAnchorElement) {
      seen.__saved.push({
        href: this.getAttribute("href"),
        download: this.getAttribute("download") ?? "absent",
      });
    };
  });
  await page
    .locator(".songs tbody tr")
    .first()
    .locator("button.song-title")
    .focus();
  await page.keyboard.press("Shift+F10");
  const menu = page.getByRole("menu", { name: "Actions for Army Of Me" });
  await menu.getByRole("menuitem", { name: "Download" }).click();
  await expect(menu).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { __saved: unknown[] }).__saved,
      ),
    )
    .toEqual([{ href: "/api/v2/stream/ticket-9?download=1", download: "" }]);
  // Nothing is left in the document once the link has been followed.
  await expect(page.locator("a[download]")).toHaveCount(0);

  // A ticket that cannot be minted says so, rather than a click doing
  // nothing; and saves nothing.
  await page.route("**/api/v2/tracks/song-9/stream-ticket", (route) =>
    route.fulfill({ status: 503, body: "" }),
  );
  await page.keyboard.press("Shift+F10");
  await menu.getByRole("menuitem", { name: "Download" }).click();
  const failed = page
    .getByRole("alert")
    .filter({ hasText: "The download could not start" });
  await expect(failed).toHaveText("The download could not start. Try again.");
  await failed.getByRole("button", { name: "Dismiss" }).click();
  await expect(failed).toHaveCount(0);
  expect(
    await page.evaluate(
      () => (window as unknown as { __saved: unknown[] }).__saved.length,
    ),
  ).toBe(1);
});

/**
 * The editor's whole job, seen from the wire. The body must name the field that
 * changed and the list handed back to the file, and nothing else — above all
 * not the comment another client added, which a body sending the whole form
 * would have erased before #177 and would pin now.
 */
test("corrects a track's tags and sends only what changed", async ({
  page,
}) => {
  await page.goto("/tracks/song-9/edit");
  await expect(
    page.getByRole("heading", { name: "Correct tags" }),
  ).toBeVisible();

  const title = page.getByLabel("Title", { exact: true });
  await expect(title).toHaveValue("Army Of Me");
  // Provenance is shown here and nowhere else: the corrected value, and what
  // the file says beneath it.
  // A tagger's comment keeps both of its lines: an `<input>` would flatten it.
  await expect(page.getByLabel("Comment", { exact: true })).toHaveValue(
    "Remastered\nBonus edition",
  );
  await expect(page.getByText("The file says nothing here")).toBeVisible();

  await title.fill("Army of Me");
  await page
    .getByRole("button", { name: /^Restore the file.s value: Artists$/ })
    .click();
  await expect(page.getByLabel("Artists", { exact: true })).toBeDisabled();

  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByText("Saved.")).toBeVisible();
  expect(mock.corrections).toEqual([{ title: "Army of Me", artists: null }]);

  // Read back, not assumed. The form now stands on the corrections the server
  // holds: the artists are the file's again, and no longer corrected.
  await expect(title).toHaveValue("Army of Me");
  const artists = page.getByLabel("Artists", { exact: true });
  await expect(artists).toBeEnabled();
  await expect(artists).toHaveValue("Björk");
  await expect(
    page.getByRole("button", { name: /^Restore the file.s value: Artists$/ }),
  ).toHaveCount(0);

  await expectNoWcagViolations(page);
});

/**
 * Offered where the server would accept it, and nowhere else. The listener half
 * is not vacuous: the album waits for the list of libraries before it renders,
 * so the rows below exist only once the role that decides them is known.
 */
test("offers a tag correction only to an owner or a manager", async ({
  page,
}) => {
  await page.goto("/albums/album-2");
  await expect(page.getByRole("heading", { name: "Vespertine" })).toBeVisible();
  const links = page.getByRole("link", { name: /^Correct tags: / });
  await expect(links).toHaveCount(3);
  await expect(links.first()).toHaveAttribute("href", "/tracks/song-1/edit");

  // A manager may correct as well: the other half of the pair the server
  // names, and the half a regression to owner-only would drop.
  mock.libraries = [{ ...library("library-1", "Ma musique"), role: "manager" }];
  await page.reload();
  await expect(page.getByRole("heading", { name: "Vespertine" })).toBeVisible();
  await expect(links).toHaveCount(3);

  mock.libraries = [
    { ...library("library-1", "Ma musique"), role: "listener" },
  ];
  await page.reload();
  await expect(page.getByRole("heading", { name: "Vespertine" })).toBeVisible();
  // The row's own marks cell, where the link would be. Once its rating is in
  // the page, the absence of the link is an answer and not a race.
  await expect(
    page.getByRole("group", { name: "Rating: Hidden Place" }),
  ).toBeAttached();
  await expect(links).toHaveCount(0);
});

/**
 * The tag editor is reached from somewhere — a song row, the track menu — and
 * its arrow goes back there. Opened from a typed address, with nowhere to go
 * back to, it lands on the songs list rather than off the site.
 */
test("goes back from the tag editor to where it was opened", async ({
  page,
}) => {
  // Opened from an album, the arrow goes back to the album, not to a list
  // it would also have landed on had it ignored where the visitor came from.
  await page.route("**/api/v2/albums/album-2", (route) =>
    route.fulfill({ json: { ...albumDetail, songs: [mock.correctable.song] } }),
  );
  await page.goto("/albums/album-2");
  await page.getByRole("link", { name: "Correct tags: Army Of Me" }).click();
  await expect(page).toHaveURL(/\/tracks\/song-9\/edit$/);
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await expect(page).toHaveURL(/\/albums\/album-2$/);

  // A full load starts the router's history afresh: with nowhere to go back
  // to, the arrow lands on the songs list, not on the album the browser
  // still holds behind this page.
  await page.goto("/tracks/song-9/edit");
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await expect(page).toHaveURL(/\/songs$/);
});

test("keeps an album, the player bar and a track's properties readable in the light", async ({
  page,
}) => {
  await chooseLightTheme(page);
  await page.goto("/albums/album-2");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "default");
  await expect(page.locator(".album-hero h2")).toHaveText("Vespertine");
  await page.locator(".album-actions .album-play").click();
  await expect(page.locator(".player-track strong")).toHaveText("Hidden Place");
  await expectNoWcagViolations(page);

  await page.route("**/api/v2/songs?*", (route) =>
    route.fulfill({ json: [mock.correctable.song] }),
  );
  await page.route("**/api/v2/tracks/song-9", (route) =>
    route.fulfill({
      json: {
        ...mock.correctable.song,
        relative_path: "Björk/Post/09 Army Of Me.flac",
      },
    }),
  );
  await page.goto("/songs");
  await page
    .locator(".songs tbody tr")
    .first()
    .locator("button.song-title")
    .focus();
  await page.keyboard.press("Shift+F10");
  await expect(
    page.getByRole("menu", { name: "Actions for Army Of Me" }),
  ).toBeVisible();
  await expectNoWcagViolations(page);
  await page.getByRole("menuitem", { name: "Properties" }).click();
  await expect(page.getByRole("dialog", { name: "Army Of Me" })).toBeVisible();
  await expectNoWcagViolations(page);
});
