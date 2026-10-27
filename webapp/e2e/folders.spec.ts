// Folders: filing tracks, uploading into a folder, the folder picker.

import { expect, test } from "@playwright/test";

import {
  chooseLightTheme,
  expectNoWcagViolations,
} from "./fixtures/accessibility";
import { resetAndMockApi } from "./fixtures/api";
import { library, song } from "./fixtures/catalogue";
import { serveFiledFolders } from "./fixtures/folders";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

/**
 * Filing, end to end on the screen: a folder created, a track filed from its
 * menu — the keyboard's way — another dragged onto a folder, the folder
 * renamed and deleted, and its tracks back among the unfiled. No control
 * here claims to touch a file, and none does: every write is to the folder
 * routes.
 */
test("files tracks into folders from the menu and by dragging", async ({
  page,
}, testInfo) => {
  const songs = [
    song(1, "Hidden Place", 0, false),
    song(2, "Cocoon", 0, false),
    song(3, "Pagan Poetry", 0, false),
  ];
  const state = await serveFiledFolders(page, songs);

  await page.goto("/folders");
  await expect(page.getByRole("heading", { name: "Folders" })).toBeVisible();
  // WaveFlow's list, not a folder of the owner's: kept apart from them, and
  // it says so.
  const kept = page.getByRole("list", { name: "Kept by WaveFlow" });
  await expect(kept.getByRole("link", { name: /To sort/ })).toContainText(
    "3 tracks",
  );
  await expect(kept.getByRole("link", { name: /To sort/ })).toContainText(
    "Automatic",
  );

  // A folder, and a second one refused for a name already taken.
  await page.getByRole("button", { name: "New folder" }).click();
  await page.getByLabel("Folder name").fill("Rock");
  await page.getByRole("button", { name: "Create" }).click();
  await expect(page.getByRole("status")).toHaveText("“Rock” created.");
  await page.getByRole("button", { name: "New folder" }).click();
  await page.getByLabel("Folder name").fill("ROCK");
  await page.getByRole("button", { name: "Create" }).click();
  await expect(
    page
      .getByRole("alert")
      .filter({ hasText: "A folder of that name is already here." }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Cancel" }).click();

  // From the menu: the unfiled tracks, the first one filed into Rock.
  await kept.getByRole("link", { name: /To sort/ }).click();
  await expect(page.getByRole("heading", { name: "To sort" })).toBeVisible();
  const rows = page.locator(".songs tbody tr");
  await expect(rows).toHaveCount(3);
  await rows.first().locator("button.song-title").focus();
  await page.keyboard.press("Shift+F10");
  await page.getByRole("menuitem", { name: "Move to folder…" }).click();
  const picker = page.getByRole("dialog", {
    name: "Move “Hidden Place” to a folder",
  });
  await expect(picker).toBeVisible();
  await expectNoWcagViolations(page);
  await picker.getByRole("button", { name: "Rock" }).click();
  await expect(picker).toBeHidden();
  await expect(rows).toHaveCount(2);
  expect(state.writes.at(-1)).toEqual({
    method: "PUT",
    path: "placements",
    body: { track_ids: ["song-1"], folder_id: "folder-1" },
  });

  // Unfiled lists tracks and says how to file them; no folder card sits
  // there, where it would read as a folder inside "Unfiled".
  await expect(page.getByRole("list", { name: "Folders" })).toHaveCount(0);
  await expect(
    page.getByText("To file a track, open its menu", { exact: false }),
  ).toBeVisible();

  // Inside the folder: a second track, a sub-folder, and a drag onto it. A
  // drag is a mouse gesture; the phone files from the menu, already proven.
  state.placed.set("song-2", "folder-1");
  await page.goto("/folders?folder=folder-1");
  await expect(page.getByRole("heading", { name: "Rock" })).toBeVisible();
  await expect(rows).toHaveCount(2);
  await expect(
    page.getByText("Folders that exist only in WaveFlow", { exact: false }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "New folder" }).click();
  await page.getByLabel("Folder name").fill("Indie");
  await page.getByRole("button", { name: "Create" }).click();
  await expect(page.getByRole("status")).toHaveText("“Indie” created.");
  if (testInfo.project.name === "desktop") {
    await rows
      .filter({ hasText: "Cocoon" })
      .dragTo(
        page
          .getByRole("list", { name: "Folders" })
          .getByRole("link", { name: /Indie/ }),
      );
    await expect(page.getByRole("status")).toHaveText(
      "1 track filed in Indie.",
    );
    await expect(rows).toHaveCount(1);
  } else {
    state.placed.set("song-2", "folder-2");
  }

  // Renamed, then deleted with its sub-folder: both tracks come back unfiled.
  await page.getByRole("button", { name: "Rename" }).click();
  await page.getByLabel("New name").fill("Rock & Roll");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(
    page.getByRole("heading", { name: "Rock & Roll" }),
  ).toBeVisible();
  page.once("dialog", (dialog) => void dialog.accept());
  await page.getByRole("button", { name: "Delete" }).click();
  await expect(page.getByRole("heading", { name: "Folders" })).toBeVisible();
  await expect(page.getByRole("status")).toHaveText(
    "Folder deleted: 2 tracks are back in To sort.",
  );
  await expect(kept.getByRole("link", { name: /To sort/ })).toContainText(
    "3 tracks",
  );
  await expectNoWcagViolations(page);
});

/** A listener walks the filed tree and is offered nothing that writes. */
test("shows a listener the filed tree with nothing to change", async ({
  page,
}) => {
  mock.libraries = [
    { ...library("library-1", "Ma musique"), role: "listener" },
  ];
  const songs = [song(1, "Hidden Place", 0, false)];
  const state = await serveFiledFolders(page, songs);
  state.folders.push({ id: "folder-1", parent_id: null, name: "Rock" });

  await page.goto("/folders?folder=unfiled");
  await expect(page.locator(".songs tbody tr")).toHaveCount(1);
  for (const name of ["New folder", "Rename", "Delete", "Move…"]) {
    await expect(page.getByRole("button", { name })).toHaveCount(0);
  }
  // A reader is told what Unfiled is, not how to file, and rows do not drag.
  await expect(
    page.getByText("WaveFlow keeps this list itself", { exact: false }),
  ).toBeVisible();
  await expect(page.locator(".songs tbody tr").first()).not.toHaveAttribute(
    "draggable",
    "true",
  );
  await page.locator(".songs tbody tr button.song-title").first().focus();
  await page.keyboard.press("Shift+F10");
  await expect(
    page.getByRole("menuitem", { name: "Properties" }),
  ).toBeVisible();
  await expect(
    page.getByRole("menuitem", { name: "Move to folder…" }),
  ).toHaveCount(0);
});

/**
 * Received first, filed second (RFC-012 decision 7): an upload started from a
 * folder commits as any other, then files the track it became.
 */
test("files a received track into the folder the upload started from", async ({
  page,
}) => {
  mock.libraries = [
    { ...library("library-1", "Ma musique"), accepts_uploads: true },
  ];
  const state = await serveFiledFolders(page, []);
  state.folders.push({ id: "folder-1", parent_id: null, name: "Rock" });

  await page.goto("/upload?folder=folder-1");
  await expect(
    page.getByText("Received files go into the folder Rock."),
  ).toBeVisible();
  await page.getByLabel("Choose audio files").setInputFiles({
    name: "Army of Me.flac",
    mimeType: "audio/flac",
    buffer: Buffer.from(Array.from({ length: 20 }, (_, i) => i)),
  });
  await expect(page.getByText("Added to the library")).toBeVisible();
  expect(mock.uploadCommits).toEqual(["upload-1"]);
  expect(state.writes).toEqual([
    {
      method: "PUT",
      path: "placements",
      body: { track_ids: ["song-uploaded"], folder_id: "folder-1" },
    },
  ]);
});

/**
 * Received is not undone by a filing that fails: the commit stands, and the
 * line says the track arrived unfiled rather than claiming it was filed.
 */
test("keeps a received track when filing it fails, and says so", async ({
  page,
}) => {
  mock.libraries = [
    { ...library("library-1", "Ma musique"), accepts_uploads: true },
  ];
  const state = await serveFiledFolders(page, []);
  state.folders.push({ id: "folder-1", parent_id: null, name: "Rock" });
  await page.route("**/api/v2/libraries/library-1/placements", (route) =>
    route.fulfill({ status: 422, json: { error: "validation" } }),
  );

  await page.goto("/upload?folder=folder-1");
  await expect(
    page.getByText("Received files go into the folder Rock."),
  ).toBeVisible();
  await page.getByLabel("Choose audio files").setInputFiles({
    name: "Army of Me.flac",
    mimeType: "audio/flac",
    buffer: Buffer.from(Array.from({ length: 20 }, (_, i) => i)),
  });
  await expect(
    page.getByText("Added to the library, in To sort"),
  ).toBeVisible();
  expect(mock.uploadCommits).toEqual(["upload-1"]);
});

/**
 * No file is taken before the folder is known, and a folder that is gone is
 * said before anything is sent: either way a file would have arrived unfiled
 * under a page that promised a folder.
 */
test("waits for the folder before taking files, and says when it is gone", async ({
  page,
}) => {
  mock.libraries = [
    { ...library("library-1", "Ma musique"), accepts_uploads: true },
  ];
  let release = () => {};
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route("**/api/v2/libraries/library-1/folders", async (route) => {
    await held;
    await route.fulfill({ json: { folders: [], unfiled_track_count: 0 } });
  });
  try {
    await page.goto("/upload?folder=folder-gone");
    await expect(page.getByLabel("Choose audio files")).toBeDisabled();
  } finally {
    release();
  }
  await expect(page.getByLabel("Choose audio files")).toBeEnabled();
  await expect(page.getByRole("status")).toHaveText(
    "The folder this upload was opened for cannot be found: received files will go to To sort.",
  );
});

/**
 * The root lists folders, not tracks. Coming back to it from the unfiled
 * tracks must not leave those tracks behind in its controls.
 */
test("offers no tracks at the root after visiting the unfiled ones", async ({
  page,
}) => {
  await serveFiledFolders(page, [song(1, "Hidden Place", 0, false)]);
  await page.goto("/folders?folder=unfiled");
  await expect(page.locator(".songs tbody tr")).toHaveCount(1);
  const controls = page.locator(".page-controls");
  await expect(controls.getByRole("button", { name: /Play/ })).toBeVisible();
  await page
    .getByRole("navigation", { name: "Folder path" })
    .getByRole("link", { name: "Folders", exact: true })
    .click();
  await expect(page.getByRole("heading", { name: "Folders" })).toBeVisible();
  await expect(controls.getByRole("button", { name: /Play/ })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Load more" })).toHaveCount(0);
});

/**
 * A tree that could not be read says nothing about the folder: it is not
 * "missing", and no file is taken until a reading works.
 */
test("takes no file while the folder cannot be checked", async ({ page }) => {
  mock.libraries = [
    { ...library("library-1", "Ma musique"), accepts_uploads: true },
  ];
  let answers = 0;
  await page.route("**/api/v2/libraries/library-1/folders", (route) => {
    answers += 1;
    return answers === 1
      ? route.fulfill({ status: 500, json: { error: "boom" } })
      : route.fulfill({
          json: {
            folders: [
              { id: "folder-1", parent_id: null, name: "Rock", track_count: 0 },
            ],
            unfiled_track_count: 0,
          },
        });
  });

  await page.goto("/upload?folder=folder-1");
  const unchecked = page.getByRole("alert").filter({
    hasText: "could not be checked",
  });
  await expect(unchecked).toBeVisible();
  await expect(page.getByLabel("Choose audio files")).toBeDisabled();
  await expect(page.getByText("cannot be found", { exact: false })).toHaveCount(
    0,
  );

  await unchecked.getByRole("button", { name: "Check again" }).click();
  await expect(
    page.getByText("Received files go into the folder Rock."),
  ).toBeVisible();
  await expect(page.getByLabel("Choose audio files")).toBeEnabled();
});

/**
 * The upload screen goes back where the upload was started. Reached by a
 * full load, with nothing behind it, it goes to the folder it files into —
 * or home, when it files into none.
 */
test("goes back from the upload screen to the folder it was opened for", async ({
  page,
}) => {
  mock.libraries = [
    { ...library("library-1", "Ma musique"), accepts_uploads: true },
  ];
  const state = await serveFiledFolders(page, []);
  state.folders.push({ id: "folder-1", parent_id: null, name: "Rock" });
  const back = page.getByRole("button", { name: "Back", exact: true });

  await page.goto("/folders?folder=folder-1");
  await page.getByRole("link", { name: "Upload here" }).click();
  await expect(
    page.getByText("Received files go into the folder Rock."),
  ).toBeVisible();
  await back.click();
  await expect(page.getByRole("heading", { name: "Rock" })).toBeVisible();

  await page.goto("/upload?folder=folder-1");
  await back.click();
  await expect(page).toHaveURL(/\/folders\?folder=folder-1$/);

  await page.goto("/upload");
  await back.click();
  await expect(page).toHaveURL(/\/$/);
});

/**
 * A form opened for one folder stays with it. The page stays mounted while
 * only the folder in the address changes; carried along, a rename would rename
 * the folder navigated to with the old one's name, and a "new folder" opened
 * at the root would create one from Unfiled.
 */
test("closes a folder's form when another folder is opened", async ({
  page,
}) => {
  const state = await serveFiledFolders(page, []);
  state.folders.push(
    { id: "folder-1", parent_id: null, name: "Rock" },
    { id: "folder-2", parent_id: "folder-1", name: "Indie" },
  );

  await page.goto("/folders?folder=folder-1");
  await page.getByRole("button", { name: "Rename" }).click();
  await expect(page.getByLabel("New name")).toHaveValue("Rock");
  await page
    .getByRole("list", { name: "Folders" })
    .getByRole("link", { name: /Indie/ })
    .click();
  await expect(page.getByRole("heading", { name: "Indie" })).toBeVisible();
  await expect(page.getByLabel("New name")).toHaveCount(0);
  await page.getByRole("button", { name: "Rename" }).click();
  await expect(page.getByLabel("New name")).toHaveValue("Indie");

  await page.goto("/folders");
  await page.getByRole("button", { name: "New folder" }).click();
  await expect(page.getByLabel("Folder name")).toBeVisible();
  await page
    .getByRole("list", { name: "Kept by WaveFlow" })
    .getByRole("link", { name: /To sort/ })
    .click();
  await expect(page.getByRole("heading", { name: "To sort" })).toBeVisible();
  await expect(page.getByLabel("Folder name")).toHaveCount(0);
  expect(state.writes).toEqual([]);
});

test("keeps the folders and the folder picker readable in the light", async ({
  page,
}) => {
  await serveFiledFolders(page, [song(1, "Hidden Place", 0, false)]);
  await chooseLightTheme(page);
  await page.goto("/folders");
  const cards = page.getByRole("list", { name: "Kept by WaveFlow" });
  await expect(cards.getByRole("link", { name: /To sort/ })).toBeVisible();
  await expectNoWcagViolations(page);

  await cards.getByRole("link", { name: /To sort/ }).click();
  await page
    .locator(".songs tbody tr")
    .first()
    .locator("button.song-title")
    .focus();
  await page.keyboard.press("Shift+F10");
  await page.getByRole("menuitem", { name: "Move to folder…" }).click();
  await expect(
    page.getByRole("dialog", { name: "Move “Hidden Place” to a folder" }),
  ).toBeVisible();
  await expectNoWcagViolations(page);
});
