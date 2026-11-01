// The library feed (RFC-007): an open tab follows what changes in a library.

import { expect, type Page, test } from "@playwright/test";

import { resetAndMockApi } from "./fixtures/api";
import { albums, library } from "./fixtures/catalogue";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

/** One poll of the feed, plus a margin: the client asks every fifteen seconds. */
const ONE_POLL_MS = 16_000;

const added = {
  ...albums[0],
  id: "album-3",
  title: "Homogenic",
  year: 1997,
};

const upsert = (cursor: number) => ({
  cursor,
  entity_type: "track",
  entity_id: `song-${cursor}`,
  action: "upsert",
  payload: { full_hash: "f".repeat(64) },
  changed_at: 1_700_000_000_000,
  origin_device_id: null,
});

/**
 * A feed the test writes to, and an album list it changes: what a scan does
 * on the server, as the two routes see it. Answers every `after` the client
 * sent, so where it asked from can be read back.
 */
async function serveFeed(page: Page, answer: (after: number) => unknown) {
  const asks: number[] = [];
  await page.route("**/api/v2/libraries/library-1/events*", async (route) => {
    const after = Number(
      new URL(route.request().url()).searchParams.get("after"),
    );
    asks.push(after);
    const answered = answer(after);
    if (typeof answered === "number") {
      await route.fulfill({ status: answered, json: { error: "conflict" } });
    } else {
      await route.fulfill({ json: answered });
    }
  });
  let shown = albums;
  let albumAsks = 0;
  await page.route("**/api/v2/albums*", async (route) => {
    albumAsks += 1;
    await route.fulfill({ json: shown });
  });
  return {
    asks,
    albumAsks: () => albumAsks,
    scan() {
      shown = [...albums, added];
    },
  };
}

test("an album a scan adds appears in an open tab without a reload", async ({
  page,
}) => {
  mock.libraries = [
    { ...library("library-1", "Ma musique"), events_cursor: 7 },
  ];
  let written: ReturnType<typeof upsert>[] = [];
  const feed = await serveFeed(page, (after) => {
    const events = written.filter((event) => event.cursor > after);
    return {
      events,
      next_cursor: events.at(-1)?.cursor ?? after,
      has_more: false,
      purged_through: 0,
    };
  });
  await page.clock.install();
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(page.getByText("Vespertine")).toBeVisible();

  // Nothing has changed: the tab asks from the head it was listed with, and
  // an empty page re-reads nothing.
  await page.clock.fastForward(ONE_POLL_MS);
  await expect.poll(() => feed.asks).toEqual([7]);
  const before = feed.albumAsks();
  await expect(page.getByText("Homogenic")).toHaveCount(0);

  feed.scan();
  written = [upsert(8)];
  await page.clock.fastForward(ONE_POLL_MS);
  await expect(page.getByText("Homogenic")).toBeVisible();
  expect(feed.albumAsks()).toBeGreaterThan(before);

  // And the next ask starts where that answer ended.
  await page.clock.fastForward(ONE_POLL_MS);
  await expect.poll(() => feed.asks.at(-1)).toBe(8);
});

test("a cursor older than retention re-reads the screen and resumes from the head", async ({
  page,
}) => {
  mock.libraries = [
    { ...library("library-1", "Ma musique"), events_cursor: 3 },
  ];
  const feed = await serveFeed(page, (after) =>
    after < 50
      ? 409
      : { events: [], next_cursor: after, has_more: false, purged_through: 50 },
  );
  await page.clock.install();
  await page.goto("/");
  await expect(page.getByText("Vespertine")).toBeVisible();

  // Retention cut the feed past this tab's cursor while it was open; the list
  // of libraries now names the head after the cut.
  feed.scan();
  mock.libraries = [
    { ...library("library-1", "Ma musique"), events_cursor: 50 },
  ];
  await page.clock.fastForward(ONE_POLL_MS);
  await expect(page.getByText("Homogenic")).toBeVisible();
  await page.clock.fastForward(ONE_POLL_MS);
  await expect.poll(() => feed.asks).toEqual([3, 50]);
});
