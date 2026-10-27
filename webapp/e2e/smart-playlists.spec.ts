// Smart playlists (RFC-025): the rules editor, its preview, and playing one.

import { expect, test } from "@playwright/test";

import { expectNoWcagViolations } from "./fixtures/accessibility";
import {
  LIBRARY_ID,
  resetAndMockSmartAndPodcastApi,
  state,
} from "./fixtures/smart-playlists-and-podcasts";

test.beforeEach(resetAndMockSmartAndPodcastApi);

test("writes a smart playlist's rules, previews them and keeps them", async ({
  page,
}) => {
  await page.goto("/playlists");
  const section = page.locator(".smart-playlists");
  await expect(
    section.getByRole("heading", { name: "Smart playlists" }),
  ).toBeVisible();
  await expect(section.getByText("No smart playlists yet.")).toBeVisible();

  await section.getByRole("button", { name: "New smart playlist" }).click();
  const editor = page.getByRole("form", { name: "New smart playlist" });
  await editor.getByLabel("Name", { exact: true }).fill("With a y");
  // An unfinished condition holds the form back: nothing to preview yet.
  await expect(editor.getByRole("button", { name: "Preview" })).toBeDisabled();
  await editor.getByLabel("Value of condition 1").fill("y");
  await editor.getByRole("button", { name: "Preview" }).click();
  await expect(editor.getByRole("status")).toContainText("2 tracks match");
  await expect(editor.getByRole("status")).toContainText("Army of Me");

  // A second condition, a number one: its comparisons are a number's.
  await editor.getByRole("button", { name: "Add a condition" }).click();
  await editor.getByLabel("Field of condition 2").selectOption("year");
  await expect(
    editor.getByLabel("Comparison of condition 2").locator("option"),
  ).toHaveText(["is", "is not", "is more than", "is less than", "is between"]);
  await editor.getByLabel("Comparison of condition 2").selectOption("between");
  await editor.getByLabel("Value of condition 2").fill("1990");
  await editor.getByLabel("Upper bound of condition 2").fill("1999");
  await editor.getByLabel("Sort by").selectOption("year");
  await editor.getByLabel(/^Order/).selectOption("desc");
  await editor.getByLabel("At most (tracks)").fill("50");
  await expectNoWcagViolations(page);
  await editor.getByRole("button", { name: "Save smart playlist" }).click();

  await expect(editor).toHaveCount(0);
  expect(state.smartWrites).toHaveLength(1);
  expect(state.smartWrites[0].body).toEqual({
    name: "With a y",
    public: false,
    rules: {
      match: "all",
      conditions: [
        { field: "title", operator: "contains", value: "y" },
        { field: "year", operator: "between", value: [1990, 1999] },
      ],
      sort: "year",
      order: "desc",
      limit: 50,
    },
  });
  const card = section
    .locator(".smart-playlist")
    .filter({ hasText: "With a y" });
  await expect(card).toContainText("2 tracks");

  // Edited: the stored rules come back into the form, and a save replaces
  // them whole.
  await card.getByRole("button", { name: "Edit rules" }).click();
  const again = page.getByRole("form", { name: "Edit rules" });
  await expect(again.getByLabel("Value of condition 1")).toHaveValue("y");
  await expect(again.getByLabel("Upper bound of condition 2")).toHaveValue(
    "1999",
  );
  await again.getByRole("button", { name: "Remove condition 2" }).click();
  await again.getByRole("button", { name: "Save smart playlist" }).click();
  await expect(again).toHaveCount(0);
  expect(state.smartWrites[1]).toMatchObject({
    method: "PATCH",
    body: {
      rules: {
        conditions: [{ field: "title", operator: "contains", value: "y" }],
      },
    },
  });

  await card.getByRole("button", { name: "Delete" }).click();
  await expect(card).toHaveCount(0);
  expect(state.smartWrites[2]).toEqual({ method: "DELETE", body: "smart-1" });
});

test("lists another account's smart playlist uncounted until it is opened", async ({
  page,
}) => {
  state.smart = [
    {
      id: "smart-public",
      owner_id: "user-2",
      owner_username: "guest",
      name: "Their y songs",
      comment: null,
      public: true,
      rules: {
        match: "all",
        conditions: [{ field: "title", operator: "contains", value: "y" }],
        sort: null,
        order: "asc",
        limit: null,
      },
      created_at: 1,
      updated_at: 1,
      // A listing evaluates nothing: never opened, so never counted.
      evaluated_at: null,
      song_count: null,
      duration_ms: null,
    },
  ];
  await page.goto("/playlists");
  const card = page
    .locator(".smart-playlist")
    .filter({ hasText: "Their y songs" });
  await expect(card).toContainText("Counted when opened");
  await expect(card).toContainText("By guest");
  await expect(card.getByRole("button", { name: "Edit rules" })).toHaveCount(0);
  await expect(card.getByRole("button", { name: "Delete" })).toHaveCount(0);
  await card.getByRole("button", { name: "Play" }).click();
  await expect(card).toContainText("2 tracks");
  await expect(page.locator(".player")).toContainText("Army of Me");
});

/** A smart playlist of this account's, as the listing answers it. */
const ownSmart = (
  id: string,
  name: string,
  needle: string,
  counted: number | null,
) => ({
  id,
  owner_id: "user-1",
  owner_username: "listener",
  name,
  comment: null,
  public: false,
  rules: {
    match: "all" as const,
    conditions: [{ field: "title", operator: "contains", value: needle }],
    sort: null,
    order: "asc" as const,
    limit: null,
  },
  created_at: 1,
  updated_at: 1,
  evaluated_at: counted === null ? null : 1,
  song_count: counted,
  duration_ms: counted === null ? null : counted * 200_000,
});

test("plays a smart playlist whose remembered count is zero", async ({
  page,
}) => {
  // Counted at zero when last opened; matching tracks arrived since. A count
  // is only what was true then, so it must not keep the playlist from playing.
  state.smart = [ownSmart("smart-stale", "Was empty", "y", 0)];
  await page.goto("/playlists");
  const card = page.locator(".smart-playlist").filter({ hasText: "Was empty" });
  await expect(card).toContainText("0 tracks");
  await card.getByRole("button", { name: "Play" }).click();
  await expect(page.locator(".player")).toContainText("Army of Me");
  await expect(card).toContainText("2 tracks");
});

test("starts a fresh editor for each playlist, and drops a preview that no longer applies", async ({
  page,
}) => {
  state.smart = [
    ownSmart("smart-a", "Playlist A", "y", 2),
    ownSmart("smart-b", "Playlist B", "Iso", 1),
  ];
  await page.goto("/playlists");
  const first = page
    .locator(".smart-playlist")
    .filter({ hasText: "Playlist A" });
  await first.getByRole("button", { name: "Edit rules" }).click();
  const editor = page.getByRole("form", { name: "Edit rules" });
  await editor.getByRole("button", { name: "Preview" }).click();
  await expect(editor.getByRole("status")).toContainText("2 tracks match");

  // Another playlist: its own rules, and no preview of the first's.
  const second = page
    .locator(".smart-playlist")
    .filter({ hasText: "Playlist B" });
  await second.getByRole("button", { name: "Edit rules" }).click();
  await expect(editor.getByLabel("Name", { exact: true })).toHaveValue(
    "Playlist B",
  );
  await expect(editor.getByLabel("Value of condition 1")).toHaveValue("Iso");
  await expect(editor.getByRole("status")).toHaveCount(0);

  // A preview, then a change to the rules: the preview goes.
  await editor.getByRole("button", { name: "Preview" }).click();
  await expect(editor.getByRole("status")).toContainText("1 track matches");
  await editor.getByLabel("Value of condition 1").fill("Army");
  await expect(editor.getByRole("status")).toHaveCount(0);
});

test("chooses a library for a condition, by name, and sends its id", async ({
  page,
}) => {
  await page.goto("/playlists");
  const section = page.locator(".smart-playlists");
  await section.getByRole("button", { name: "New smart playlist" }).click();
  const editor = page.getByRole("form", { name: "New smart playlist" });
  await editor.getByLabel("Name", { exact: true }).fill("From one library");
  await editor.getByLabel("Field of condition 1").selectOption("library");
  await expect(
    editor.getByLabel("Comparison of condition 1").locator("option"),
  ).toHaveText(["is", "is not"]);
  await expect(
    editor.getByRole("button", { name: "Save smart playlist" }),
  ).toBeDisabled();
  await editor
    .getByLabel("Value of condition 1")
    .selectOption({ label: "Ma musique" });
  await editor.getByRole("button", { name: "Save smart playlist" }).click();
  await expect(editor).toHaveCount(0);
  expect(state.smartWrites[0].body).toMatchObject({
    rules: {
      conditions: [{ field: "library", operator: "is", value: LIBRARY_ID }],
    },
  });
});
