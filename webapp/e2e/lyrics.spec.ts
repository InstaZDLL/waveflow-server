// The words a track travels with, the sung line, and where they came from.

import { expect, test } from "@playwright/test";

import { expectNoWcagViolations } from "./fixtures/accessibility";
import { resetAndMockApi } from "./fixtures/api";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

/**
 * The playing page, reading what the server actually sends.
 *
 * Until 2026-09-15 `LyricsList` declared a `snake_case` envelope the server has
 * never sent, so this page threw on its first property and showed nothing at
 * all. Nothing caught it because the mock in `fixtures/api.ts` was written
 * from that type. An assertion on the words themselves is what tells the two
 * shapes apart: an empty sheet renders the same "no lyrics" line under either
 * name.
 */
test("shows the words a track travels with", async ({ page }) => {
  mock.lyricSheets.set("song-1", [
    {
      displayArtist: "Rue Delacour",
      displayTitle: "Nocturne",
      lang: "eng",
      synced: false,
      line: [{ value: "the lamps come on along the quay" }],
    },
  ]);
  await page.goto("/playing");
  await expect(
    page.getByRole("listitem").filter({
      hasText: "the lamps come on along the quay",
    }),
  ).toBeVisible();
});

/**
 * The words as the subject of the screen, and what timings buy.
 *
 * A sheet that carries them gets two things the strip cannot give: the sung
 * line is held in view, and every line is a button that seeks to it. A sheet
 * without them gets neither — and says so by having no buttons at all, rather
 * than promising a jump it cannot compute.
 *
 * No audio is served in these runs, so the head stays at zero and the first
 * line is the sung one. That is enough to tell a marked line from an unmarked
 * one; what it deliberately does not claim is that pressing a line moved the
 * head, which without a stream would be a measurement of nothing.
 */
test("gives the words a view of their own, with timings where there are any", async ({
  page,
}) => {
  mock.lyricSheets.set("song-1", [
    {
      displayArtist: "Rue Delacour",
      displayTitle: "Nocturne",
      lang: "eng",
      synced: true,
      line: [
        { start: 0, value: "the lamps come on along the quay" },
        { start: 5_000, value: "and the water keeps the light" },
        { start: 12_500, value: "the lamps come on along the quay" },
        // An instrumental break: timed, and with no words to name it by.
        { start: 20_000, value: "" },
      ],
    },
  ]);

  await page.goto("/playing");
  await page.getByRole("link", { name: "Lyrics", exact: true }).click();

  const view = page.locator(".lyrics-view");
  await expect(view).toBeVisible();
  await expect(page).toHaveURL(/\?view=lyrics$/);
  // The strip is gone, not duplicated: one view at a time, or the words would
  // be on the screen twice and the sung line marked in two places.
  await expect(page.locator(".playing")).toHaveCount(0);

  // The head is at zero, so the first line is the one being sung — and a
  // refrain repeated word for word does not take the mark with it.
  const lines = view.getByRole("listitem");
  await expect(lines).toHaveCount(4);
  await expect(lines.nth(0)).toHaveAttribute("aria-current", "true");
  await expect(lines.nth(2)).not.toHaveAttribute("aria-current", "true");

  // **Every line is a button, named by its words and then the time.** The
  // words because the name must contain what is on screen — voice control
  // finds a control by what it shows — and the time because the words alone
  // say nothing about what pressing does. A blank line has no words to
  // repeat, so it keeps the time alone rather than a name that opens on a
  // comma.
  await expect(view.getByRole("button")).toHaveCount(4);
  await expect(
    view.getByRole("button", {
      name: "and the water keeps the light, jump to 0:05",
      exact: true,
    }),
  ).toBeVisible();
  // Visible, not merely present: a blank line held a collapsible space, so its
  // button had no height — reachable by the keyboard and drawn nowhere.
  await expect(
    view.getByRole("button", { name: "Jump to 0:20", exact: true }),
  ).toBeVisible();
  await expect(
    view.getByText("This sheet carries timings — press a line to jump to it."),
  ).toBeVisible();

  await expectNoWcagViolations(page);

  // **The switcher says where it is, once.** All three entries point at one
  // path, and the router counts a link carrying no search as matching every
  // search — so this read `['Detail', 'Lyrics']` before `activeOptions` was
  // made exact, and the screen claimed to be in two views at once. A count
  // rather than one attribute: the defect was an extra mark, which an
  // assertion on the right one passes straight through.
  const current = page.locator('.view-switcher a[aria-current="page"]');
  await expect(current).toHaveText("Lyrics");
  await page.goto("/playing");
  await expect(page.locator(".playing")).toBeVisible();
  await expect(current).toHaveText("Detail");

  // And a mode this build does not know is not a mode: the detail view
  // stands, rather than a blank screen or a crash.
  await page.goto("/playing?view=karaoke");
  await expect(page.locator(".playing")).toBeVisible();
  await expect(page.locator(".lyrics-view")).toHaveCount(0);
});

/**
 * A sheet that arrives after the head has already moved is still centred.
 *
 * The case a review asked about: open the lyrics view mid-track with nothing
 * cached, so the column mounts only once the sheet lands. The claim was that
 * the scroll effect would not re-run, because it watches `active` and not the
 * three-state reading beside it. It does run — `active` is derived from the
 * sheet, so it is `-1` until one arrives and cannot already name a line — and
 * this is that argument turned into a measurement rather than left as one.
 */
test("centres a sheet that lands after the head has moved", async ({
  page,
}) => {
  mock.lyricSheets.set("song-1", [
    {
      displayArtist: "Rue Delacour",
      displayTitle: "Nocturne",
      lang: "eng",
      synced: true,
      line: Array.from({ length: 40 }, (_, index) => ({
        start: index * 5_000,
        value: `late line ${index}`,
      })),
    },
  ]);
  let release = () => {};
  mock.lyricsGate = new Promise<void>((resolve) => {
    release = resolve;
  });

  try {
    await page.goto("/playing?view=lyrics");
    // Waiting, not empty: the view's frame is there and the column is not.
    await expect(page.locator(".lyrics-view")).toContainText("Loading");
    await expect(page.locator(".lyrics-view ol")).toHaveCount(0);

    // The head moves while there is no sheet to mark, so `active` stays -1
    // and the effect has nothing to do.
    for (let press = 0; press < 20; press += 1) {
      await page.keyboard.press("ArrowRight");
    }
  } finally {
    // **Released whatever happened.** An assertion failing above would leave
    // the route waiting on this promise for ever, and the context teardown
    // waiting on the route — so the test would not fail, it would hang. The
    // reset in `beforeEach` guards the tests after this one, not this one.
    //
    // The two gates added with the credential panels (`settings.spec.ts`), and
    // the canvas one (`canvas.spec.ts`), carry the same hazard and are left as
    // they are: wrapping them means restructuring three tests this branch has
    // no other reason to touch.
    release();
  }
  const column = page.locator(".lyrics-view ol");
  await expect(column).toBeVisible();
  await expect(
    page.getByRole("listitem").filter({ hasText: "late line 20" }),
  ).toHaveAttribute("aria-current", "true");
  // The line the head is on is in view, which is the whole claim.
  await expect
    .poll(() => column.evaluate((list) => list.scrollTop))
    .toBeGreaterThan(0);
});

/**
 * A sheet without timings offers no seeking, and does not pretend to follow.
 *
 * The same view, the other half of the rule: `synced` is the server's statement
 * that the lines carry starts, and everything this view adds over the strip
 * hangs off it.
 */
test("offers no seeking on a sheet that carries no timings", async ({
  page,
}) => {
  mock.lyricSheets.set("song-1", [
    {
      displayArtist: null,
      displayTitle: "Nocturne",
      lang: "eng",
      synced: false,
      line: [
        { value: "the lamps come on along the quay" },
        { value: "and the water keeps the light" },
      ],
    },
  ]);

  await page.goto("/playing?view=lyrics");

  const view = page.locator(".lyrics-view");
  await expect(view.getByRole("listitem")).toHaveCount(2);
  await expect(view.getByRole("button")).toHaveCount(0);
  await expect(
    view.getByText("This sheet carries timings — press a line to jump to it."),
  ).toHaveCount(0);
  // Nothing is marked: an unsynced sheet has no start to compare against, and
  // weighting a line would be a claim about where the track is.
  await expect(view.locator("[aria-current]")).toHaveCount(0);
});

/**
 * The strip beside the cover reads the way the lyrics view does.
 *
 * It used to mark the sung line and leave it to scroll out of its own box, and
 * no line could be pressed. Now it holds the sung line in view, and a synced
 * line is a button that seeks to it. A sheet without timings still gets no
 * buttons.
 */
test("follows the sung line in the strip beside the cover, and seeks from it", async ({
  page,
}) => {
  mock.lyricSheets.set("song-1", [
    {
      displayArtist: "Rue Delacour",
      displayTitle: "Nocturne",
      lang: "eng",
      synced: true,
      line: Array.from({ length: 40 }, (_, index) => ({
        start: index * 5_000,
        value: `strip line ${index}`,
      })),
    },
  ]);

  await page.goto("/playing");
  const strip = page.locator(".playing .lyrics");
  const column = strip.locator("ol");
  await expect(column).toBeVisible();
  // Every timed line is a button, named by its words and then the time.
  await expect(strip.getByRole("button")).toHaveCount(40);
  await expect(
    strip.getByRole("button", {
      name: "strip line 1, jump to 0:05",
      exact: true,
    }),
  ).toBeVisible();
  await expect.poll(() => column.evaluate((list) => list.scrollTop)).toBe(0);

  // The head moves well down the sheet: the sung line is marked, and the strip
  // has scrolled itself to it rather than leaving it below its own fold.
  for (let press = 0; press < 20; press += 1) {
    await page.keyboard.press("ArrowRight");
  }
  await expect(
    strip.getByRole("listitem").filter({ hasText: "strip line 20" }),
  ).toHaveAttribute("aria-current", "true");
  await expect
    .poll(() => column.evaluate((list) => list.scrollTop))
    .toBeGreaterThan(0);
  // The strip scrolls itself and nothing else: the page stays where it was.
  expect(await page.evaluate(() => window.scrollY)).toBe(0);

  // Pressing a line moves the head to it. The sung line is derived from the
  // player's position, so the mark landing on the pressed line is the head
  // having moved there — the arrow keys above go through the same `seek`.
  await strip
    .getByRole("button", { name: "strip line 5, jump to 0:25", exact: true })
    .click();
  await expect(
    strip.getByRole("listitem").filter({ hasText: /^strip line 5$/ }),
  ).toHaveAttribute("aria-current", "true");
  await expect(
    strip.getByRole("listitem").filter({ hasText: /^strip line 20$/ }),
  ).not.toHaveAttribute("aria-current", "true");

  await expectNoWcagViolations(page);

  // Without timings, the strip offers no seeking, as the lyrics view does not.
  mock.lyricSheets.set("song-1", [
    {
      displayArtist: null,
      displayTitle: "Nocturne",
      lang: "eng",
      synced: false,
      line: [
        { value: "the lamps come on along the quay" },
        { value: "and the water keeps the light" },
      ],
    },
  ]);
  await page.reload();
  await expect(strip.getByRole("listitem")).toHaveCount(2);
  await expect(strip.getByRole("button")).toHaveCount(0);
});

/**
 * The immersive view, and the two things that make it a mode rather than a trap.
 *
 * It **carries transport controls**, because it covers the player bar: a view
 * that fills the screen with a cover and leaves no way to pause is a trap. And
 * it **lives in the location**, so Back leaves the view rather than the screen
 * — which is the press everybody makes on a phone, and the whole reason the
 * mode is a search parameter instead of a `useState`.
 */
test("opens the immersive view, and Back leaves the view and not the screen", async ({
  page,
}) => {
  await page.goto("/playing");
  await page.getByRole("link", { name: "Immersive" }).click();

  const immersive = page.locator("dialog.immersive");
  await expect(immersive).toBeVisible();
  await expect(page).toHaveURL(/\?view=immersive$/);
  await expect(immersive.locator(".immersive-cover")).toBeVisible();
  // The transport, so the mode can be sat in.
  for (const name of ["Previous track", "Play", "Next track"]) {
    await expect(immersive.getByRole("button", { name })).toBeVisible();
  }

  await expectNoWcagViolations(page);

  // **Escape, which is the platform's and not ours.** `<dialog>` answers it,
  // and the navigation hangs off the close rather than off the key, so the
  // button below and this press go through one closer.
  await page.keyboard.press("Escape");
  await expect(immersive).toHaveCount(0);
  await expect(page).toHaveURL(/\/playing$/);
  await expect(page.locator(".playing")).toBeVisible();

  // **And Back does not reopen what Escape just closed.** The close replaces
  // the immersive entry rather than pushing a detail one on top of it.
  // Pushing left `/playing → ?view=immersive → /playing` in the history, so
  // this press landed back on the immersive entry and opened the view again —
  // the opposite of the reason the mode is in the location.
  await page.goBack();
  await expect(page.locator("dialog.immersive")).toHaveCount(0);

  // And Back, from inside the view, leaves the view.
  await page.getByRole("link", { name: "Immersive" }).click();
  await expect(page.locator("dialog.immersive")).toBeVisible();
  await page.goBack();
  await expect(page.locator("dialog.immersive")).toHaveCount(0);
  await expect(page).toHaveURL(/\/playing$/);
  await expect(page.locator(".playing")).toBeVisible();

  // The close button takes the same route out.
  await page.getByRole("link", { name: "Immersive" }).click();
  await page.getByRole("button", { name: "Leave the immersive view" }).click();
  await expect(page.locator("dialog.immersive")).toHaveCount(0);
  await expect(page).toHaveURL(/\/playing$/);
});

/**
 * The sung line is held in view, and only the column moves to do it.
 *
 * `scrollIntoView` scrolls every scrollable ancestor it has, the window
 * included, so each new sung line dragged the whole page back to the words —
 * and somebody reading the saved positions underneath was hauled upwards every
 * few seconds. The column scrolls itself now.
 *
 * The head is moved with the keyboard rather than by pressing a line, because
 * Playwright scrolls an element into view before clicking it and that scroll
 * would be the one under test. Twelve saved positions are there to make the
 * page taller than the viewport: without something below the fold, "the window
 * did not scroll" is true however the scrolling is done.
 */
test("holds the sung line in view without dragging the page", async ({
  page,
}) => {
  mock.lyricSheets.set("song-1", [
    {
      displayArtist: "Rue Delacour",
      displayTitle: "Nocturne",
      lang: "eng",
      synced: true,
      line: Array.from({ length: 40 }, (_, index) => ({
        start: index * 5_000,
        value: `line number ${index}`,
      })),
    },
  ]);
  mock.bookmarks = Array.from({ length: 12 }, (_, index) => ({
    position_ms: index * 1_000,
    comment: null,
    created_at: 1,
    updated_at: 1,
    song: {
      id: `saved-${index}`,
      title: `Somewhere to pick up ${index}`,
      artist: "Rue Delacour",
    },
  }));

  await page.goto("/playing?view=lyrics");
  const column = page.locator(".lyrics-view ol");
  await expect(column).toBeVisible();
  // The list the screen renders under the track, drawn by a test at last.
  await expect(page.locator(".bookmark-list li")).toHaveCount(12);
  const scrollable = await page.evaluate(
    () => document.documentElement.scrollHeight > window.innerHeight,
  );
  expect(scrollable).toBe(true);

  // **Scrolled down to the saved positions first, which is the whole case.**
  // With the column fully on screen there is nothing for an ancestor scroll to
  // do, so the page stays put however the following is written — the complaint
  // was about a reader who had scrolled away from the words and kept being
  // hauled back to them.
  await page.evaluate(() => window.scrollTo(0, 400));
  const before = await page.evaluate(() => window.scrollY);
  expect(before).toBeGreaterThan(0);

  // Twenty presses of five seconds each: the player listens on the window, and
  // a modal dialog is the only thing that would be in the way.
  for (let press = 0; press < 20; press += 1) {
    await page.keyboard.press("ArrowRight");
  }

  // The head moved, which is also the proof that a line's own press would
  // seek: both go through `player.seek`, and the position is React state
  // rather than something read back off an element that is playing nothing.
  await expect(
    page.getByRole("listitem").filter({ hasText: "line number 20" }),
  ).toHaveAttribute("aria-current", "true");

  // Smooth scrolling settles over frames, so the column is polled rather than
  // read once.
  await expect
    .poll(() => column.evaluate((list) => list.scrollTop))
    .toBeGreaterThan(0);
  // And the page did not move under the reader.
  expect(await page.evaluate(() => window.scrollY)).toBe(before);
});

test("credits LRCLIB for words the server looked up", async ({ page }) => {
  mock.lyricSheets.set("song-1", [
    {
      displayArtist: "Björk",
      displayTitle: "Army of Me",
      lang: "xxx",
      synced: true,
      line: [
        { start: 1_000, value: "If you complain once more" },
        { start: 4_500, value: "You'll meet an army of me" },
      ],
      source: "lrclib",
    },
  ]);
  await page.goto("/playing");
  await expect(page.locator(".playing .lyrics")).toContainText(
    "Lyrics from LRCLIB",
  );
  await page.getByRole("link", { name: "Lyrics", exact: true }).click();
  await expect(page.locator(".lyrics-view")).toContainText(
    "Lyrics from LRCLIB",
  );
  await expectNoWcagViolations(page);
});

test("credits nobody for words that travel with the file", async ({ page }) => {
  mock.lyricSheets.set("song-1", [
    {
      displayArtist: "Björk",
      displayTitle: "Army of Me",
      lang: "eng",
      synced: false,
      line: [{ value: "If you complain once more" }],
      source: "embedded",
    },
  ]);
  await page.goto("/playing");
  await expect(page.locator(".playing .lyrics")).toContainText(
    "If you complain once more",
  );
  await expect(page.getByText("Lyrics from LRCLIB")).toHaveCount(0);
});
