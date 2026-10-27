// Playlists: .m3u import and export, entries, another account's lists.

import { expect, test } from "@playwright/test";

import { expectNoWcagViolations } from "./fixtures/accessibility";
import { resetAndMockApi } from "./fixtures/api";
import { playlists, song } from "./fixtures/catalogue";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

/**
 * A playlist file arrives, and nothing is created until the losses are shown.
 *
 * The panel is two steps because the failure it exists to prevent is a quiet
 * one: a file from another player names paths from another filesystem, so some
 * entries will not be here, and an import that silently produced a shorter
 * playlist would leave no way to know. So the first press only asks, and the
 * report is on screen — by line, with a reason — before anything is written.
 */
test("previews an .m3u import, shows what it cannot place, then imports", async ({
  page,
}) => {
  await page.goto("/playlists");
  await expect(
    page.getByRole("heading", { name: "Bring playlists from another player" }),
  ).toBeVisible();

  // The file is chosen, and only a dry run happens.
  await page.getByLabel("Choose a playlist file").setInputFiles({
    name: "Soirée.m3u",
    mimeType: "audio/x-mpegurl",
    buffer: Buffer.from("#EXTM3U\nA/01.flac\n"),
  });
  await expect(
    page.getByText("3 of 5 entries are in your library."),
  ).toBeVisible();
  expect(mock.importAsks).toEqual([{ dryRun: true, name: "Soirée" }]);
  // The name offered is the file's own, without its extension, and editable.
  await expect(page.getByLabel("Playlist name", { exact: true })).toHaveValue(
    "Soirée",
  );

  // Both losses are named, with their line and their reason.
  await expect(page.getByText("2 entries cannot be placed:")).toBeVisible();
  const losses = page.locator(".import-unresolved li");
  await expect(losses).toHaveCount(2);
  await expect(losses.first()).toContainText("Line 7");
  await expect(losses.first()).toContainText("matches several tracks");
  await expect(losses.nth(1)).toContainText("not a file in a library");
  // And the encoding, because a file that was not UTF-8 may have mangled
  // names and the reader should know which guess was made.
  await expect(
    page.getByText(
      "The file was not UTF-8 and was read as windows-1252. Check the names below before importing.",
    ),
  ).toBeVisible();

  await expectNoWcagViolations(page);

  // Cancelling writes nothing, and asks nothing more.
  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByLabel("Playlist name", { exact: true })).toHaveCount(
    0,
  );
  expect(mock.importAsks).toHaveLength(1);

  // And the panel opens again after a cancel, on a fresh choice.
  //
  // **It does not pin the input being cleared between choices**, which is what
  // lets a real user re-choose a file they have just fixed: `setInputFiles`
  // fires a change event whether or not the file changed, so this harness
  // cannot tell the two apart. Removing the reset leaves these assertions
  // green — measured, not assumed.
  await page.getByLabel("Choose a playlist file").setInputFiles({
    name: "Soirée.m3u",
    mimeType: "audio/x-mpegurl",
    buffer: Buffer.from("#EXTM3U\nA/01.flac\n"),
  });
  await expect(page.getByLabel("Playlist name", { exact: true })).toBeVisible();
  expect(mock.importAsks).toHaveLength(2);

  // A name of the reader's choosing, then the commit.
  await page
    .getByLabel("Playlist name", { exact: true })
    .fill("Evening from elsewhere");
  await page.getByRole("button", { name: "Import", exact: true }).click();
  await expect(page.getByLabel("Playlist name", { exact: true })).toHaveCount(
    0,
  );
  expect(mock.importAsks[2]).toEqual({
    dryRun: false,
    name: "Evening from elsewhere",
  });
});

/**
 * Saving a playlist as a file, which is the other half of arriving with one.
 */
test("saves a playlist as an .m3u8 the next player can read", async ({
  page,
}) => {
  await page.goto("/playlists");
  await expect(page.getByRole("heading", { name: "Evening" })).toBeVisible();

  const download = page.waitForEvent("download");
  await page.getByRole("button", { name: "Save as .m3u8" }).click();
  const saved = await download;
  // Named after the playlist, because a folder of "playlist.m3u8" is no use.
  expect(saved.suggestedFilename()).toBe("Evening.m3u8");
  expect(mock.exportAsks).toEqual(["/api/v2/playlists/playlist-1/export"]);
});

/**
 * A playlist holds a position, not a set: the same track can be on it twice.
 * Each row's rating is a radio group named after the row, and a name built
 * from the track alone put both copies in one group — the browser keeps one
 * checked radio per group, so the first copy showed no rating at all.
 */
test("shows the same track twice in a playlist, each with its rating", async ({
  page,
}) => {
  // `?*`: the page asks for the public playlists of other accounts too
  // (RFC-020), and a pattern without it matches no address with a query.
  await page.route("**/api/v2/playlists?*", (route) =>
    route.fulfill({
      json: [
        {
          ...playlists[0],
          songs: [
            song(1, "Hidden Place", 5, true),
            song(2, "Cocoon", 0, false),
            song(1, "Hidden Place", 5, true),
          ],
        },
      ],
    }),
  );
  await page.goto("/playlists");
  await expect(page.getByRole("heading", { name: "Evening" })).toBeVisible();

  const copies = page.getByRole("group", { name: "Rating: Hidden Place" });
  await expect(copies).toHaveCount(2);
  await expect(
    copies.first().getByRole("radio", { name: "5 stars" }),
  ).toBeChecked();
  await expect(
    copies.last().getByRole("radio", { name: "5 stars" }),
  ).toBeChecked();
});

/**
 * Another account's public playlist is listed apart from this account's own,
 * under its owner's name, and offers nothing that would change it: the server
 * answers anyone but the owner 404 (RFC-020 decision 2).
 */
test("lists another account's public playlist apart, and read-only", async ({
  page,
}) => {
  await page.route("**/api/v2/playlists?*", (route) =>
    route.fulfill({
      json: [
        playlists[0],
        {
          ...playlists[0],
          id: "playlist-2",
          owner_id: "user-2",
          owner_username: "guest",
          name: "Guest picks",
          public: true,
        },
      ],
    }),
  );
  await page.goto("/playlists");
  const others = page.getByRole("region", { name: "From other accounts" });
  await expect(
    others.getByRole("heading", { name: "Guest picks" }),
  ).toBeVisible();
  await expect(others.getByText("By guest")).toBeVisible();
  await expect(others.getByRole("button", { name: "Delete" })).toHaveCount(0);
  await expect(others.getByRole("checkbox")).toHaveCount(0);
  // The account's own keeps every control, the publishing switch included.
  await expect(
    page.getByRole("checkbox", { name: "Visible to every account" }),
  ).toHaveCount(1);
});

/**
 * A playlist's owner moves an entry and removes one, each named by the
 * position shown (RFC-020, what was still open): a move sends the track it
 * believes is there, so a list changed elsewhere is refused rather than
 * edited in the wrong place.
 */
test("moves and removes a playlist entry by the position it is shown at", async ({
  page,
}) => {
  const asks: Array<[string, unknown]> = [];
  const listed = {
    ...playlists[0],
    songs: [
      song(1, "Hidden Place", 5, true),
      song(2, "Cocoon", 0, false),
      song(3, "Aurora", 0, false),
    ],
  };
  await page.route("**/api/v2/playlists?*", (route) =>
    route.fulfill({ json: [listed] }),
  );
  await page.route("**/api/v2/playlists/playlist-1/move", async (route) => {
    asks.push(["move", route.request().postDataJSON()]);
    await route.fulfill({ json: listed });
  });
  await page.route("**/api/v2/playlists/playlist-1/remove", async (route) => {
    asks.push(["remove", route.request().postDataJSON()]);
    await route.fulfill({ json: listed });
  });
  await page.route("**/api/v2/playlists/playlist-1", async (route) => {
    asks.push([route.request().method(), route.request().postDataJSON()]);
    await route.fulfill({ json: listed });
  });
  await page.goto("/playlists");
  await expect(page.getByRole("heading", { name: "Evening" })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Move Hidden Place up" }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Move Aurora down" }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Move Cocoon down" }).click();
  await expect.poll(() => asks.length).toBe(1);
  await page
    .getByRole("button", { name: "Remove Aurora from this playlist" })
    .click();
  await expect.poll(() => asks.length).toBe(2);
  expect(asks).toEqual([
    ["move", { from: 1, to: 2, track_id: "song-2" }],
    ["remove", { position: 2, track_id: "song-3" }],
  ]);
  await expectNoWcagViolations(page);
});

/** Another account's public playlist offers none of an owner's controls. */
test("offers no move or removal on another account's playlist", async ({
  page,
}) => {
  await page.route("**/api/v2/playlists?*", (route) =>
    route.fulfill({
      json: [
        {
          ...playlists[0],
          id: "playlist-2",
          owner_id: "user-2",
          owner_username: "guest",
          name: "Guest picks",
          public: true,
          songs: [
            song(1, "Hidden Place", 5, true),
            song(2, "Cocoon", 0, false),
          ],
        },
      ],
    }),
  );
  await page.goto("/playlists");
  await expect(
    page.getByRole("heading", { name: "Guest picks" }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: /^Move / })).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: /from this playlist$/ }),
  ).toHaveCount(0);
});

/**
 * One entry edit at a time: the controls stay held until the edited list
 * has been read again, so a second click cannot name positions in a list the
 * first edit already changed (review of #335).
 */
test("holds a playlist's entry controls until the edited list is read again", async ({
  page,
}) => {
  const listed = {
    ...playlists[0],
    songs: [
      song(1, "Hidden Place", 5, true),
      song(2, "Cocoon", 0, false),
      song(3, "Aurora", 0, false),
    ],
  };
  await page.route("**/api/v2/playlists?*", (route) =>
    route.fulfill({ json: [listed] }),
  );
  let answer: () => void = () => undefined;
  const held = new Promise<void>((resolve) => {
    answer = resolve;
  });
  let moves = 0;
  await page.route("**/api/v2/playlists/playlist-1/move", async (route) => {
    moves += 1;
    await held;
    await route.fulfill({ json: listed });
  });
  await page.goto("/playlists");
  await page.getByRole("button", { name: "Move Cocoon down" }).click();
  await expect.poll(() => moves).toBe(1);
  for (const name of [
    "Move Cocoon up",
    "Move Hidden Place down",
    "Remove Aurora from this playlist",
  ]) {
    await expect(page.getByRole("button", { name })).toBeDisabled();
  }
  answer();
  await expect(
    page.getByRole("button", { name: "Move Cocoon up" }),
  ).toBeEnabled();
  expect(moves).toBe(1);
});
