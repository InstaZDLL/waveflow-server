// Podcasts (RFC-025): subscribing, downloading, and episodes in the player bar.

import { expect, type Page, test } from "@playwright/test";

import { expectNoWcagViolations } from "./fixtures/accessibility";
import { PIXEL } from "./fixtures/media";
import { navigateWithin } from "./fixtures/radio";
import {
  type Episode,
  episode,
  resetAndMockSmartAndPodcastApi,
  state,
} from "./fixtures/smart-playlists-and-podcasts";

test.beforeEach(resetAndMockSmartAndPodcastApi);

test("subscribes to a podcast, downloads, plays and keeps the place", async ({
  page,
}) => {
  await page.goto("/podcasts");
  await expect(page.getByRole("heading", { name: "Podcasts" })).toBeVisible();
  await expect(page.getByText("No podcasts yet.")).toBeVisible();
  // Whoever subscribes is told, at the field, that the show is not theirs alone.
  await expect(page.getByLabel("Feed address")).toHaveAccessibleDescription(
    /^Visible to every account/,
  );
  await page
    .getByLabel("Feed address")
    .fill("https://feeds.example.org/night.xml");
  await page.getByRole("button", { name: "Subscribe" }).click();
  // The route sees the write after the click returns, not before it: polled.
  await expect
    .poll(() => state.podcastWrites)
    .toEqual(["subscribe https://feeds.example.org/night.xml"]);

  const episodes = page.locator(".podcast-episodes");
  await expect(
    episodes.getByRole("heading", { name: "Night Shift" }),
  ).toBeVisible();
  const first = episodes.locator("li").filter({ hasText: "First night" });
  await expect(first).toContainText("not downloaded");
  await first.getByRole("button", { name: "Download" }).click();
  await expect.poll(() => state.podcastWrites).toContain("download episode-1");
  await expect(first).toContainText("ready");

  // The saved place is shown, and playing resumes from it.
  const second = episodes.locator("li").filter({ hasText: "Second night" });
  await expect(second).toContainText("resumes at 0:30");
  await expectNoWcagViolations(page);
  await first.getByRole("button", { name: "Play First night" }).click();
  await expect(
    first.getByRole("button", { name: "Pause First night" }),
  ).toBeVisible();
  await first.getByRole("button", { name: "Pause First night" }).click();
  // Pausing writes the place through the bookmark route, under the episode's id.
  await expect
    .poll(() => state.bookmarkWrites.map((write) => write.id))
    .toContain("episode-1");

  await page.getByRole("button", { name: "Refresh every feed" }).click();
  await expect.poll(() => state.podcastWrites).toContain("refresh");
});

/**
 * Every media element asked to play, kept on the window: the player's
 * elements are never in the document, so this is the only way to reach them.
 */
async function recordPlayedElements(page: Page) {
  await page.addInitScript(() => {
    const played = new Set<HTMLMediaElement>();
    (window as unknown as { __played: Set<HTMLMediaElement> }).__played =
      played;
    const play = HTMLMediaElement.prototype.play;
    HTMLMediaElement.prototype.play = function () {
      played.add(this);
      return play.call(this);
    };
  });
}

/** The element playing the episode served under this ticket, as seen now. */
function episodeElement(page: Page, ticket: string) {
  return page.evaluate((ticket) => {
    const element = [
      ...(window as unknown as { __played: Set<HTMLMediaElement> }).__played,
    ].find((candidate) => candidate.currentSrc.endsWith(`/${ticket}`));
    return element
      ? { paused: element.paused, currentTime: element.currentTime }
      : null;
  }, ticket);
}

const ready = { status: "completed" as const, suffix: "wav", size: 8044 };

const channelOf = (id: string, title: string, episodes: Episode[]) => ({
  id,
  url: null,
  title,
  description: null,
  site_url: null,
  image_url: null,
  artwork_hash: null,
  status: "completed",
  error: null,
  created_at: 1,
  refreshed_at: 1,
  episode_count: episodes.length,
  episodes,
});

/**
 * An episode plays in the player bar and not on the page (the page used to
 * own a private element and pause it on the way out). So it survives leaving
 * the page, the bar shows where it is and seeks in it, and the bookmark is
 * written from the bar as it was from the page — then read back to resume.
 */
test("plays an episode in the bar, across pages, and resumes it from its bookmark", async ({
  page,
}) => {
  await recordPlayedElements(page);
  state.channels = [
    channelOf("channel-1", "Night Shift", [
      // Thirty seconds of sound, as the route serves it: the duration the
      // feed announced is replaced by the file's once it loads.
      episode("episode-1", "First night", { ...ready, duration_ms: 30_000 }),
    ]),
  ];
  await page.goto("/podcasts");
  await page.getByRole("button", { name: "Episodes" }).click();
  const first = page
    .locator(".podcast-episodes li")
    .filter({ hasText: "First night" });
  await first.getByRole("button", { name: "Play First night" }).click();

  const bar = page.locator(".player");
  await expect(bar.locator(".player-track strong")).toHaveText("First night");
  await expect(bar.locator(".player-track")).toContainText("Night Shift");
  await expect(bar.getByRole("button", { name: "Pause" })).toBeVisible();
  await expect
    .poll(async () => (await episodeElement(page, "ticket-episode-1"))?.paused)
    .toBe(false);

  // Away from the page that started it: still playing, still in the bar.
  await navigateWithin(page, "Playlists");
  await expect(page.locator(".smart-playlists")).toBeVisible();
  await expect(bar.locator(".player-track strong")).toHaveText("First night");
  await expect
    .poll(
      async () =>
        (await episodeElement(page, "ticket-episode-1"))?.currentTime ?? 0,
    )
    .toBeGreaterThan(0.2);
  expect((await episodeElement(page, "ticket-episode-1"))?.paused).toBe(false);
  // An episode has a line and a scrubber, and nothing the queue has: no next,
  // no previous, no shuffle, no repeat.
  const seek = bar.getByRole("slider", { name: "Seek" });
  await expect(seek).toBeVisible();
  await expect(bar.getByRole("button", { name: "Next track" })).toHaveCount(0);
  await expect(bar.getByRole("button", { name: "Previous track" })).toHaveCount(
    0,
  );
  await expect(bar.getByRole("button", { name: "Shuffle" })).toHaveCount(0);
  await expectNoWcagViolations(page);

  // Seeking from the bar moves the episode, not a track.
  await seek.fill("12");
  await seek.dispatchEvent("mouseup");
  await expect
    .poll(
      async () =>
        (await episodeElement(page, "ticket-episode-1"))?.currentTime ?? 0,
    )
    .toBeGreaterThanOrEqual(12);

  // Pausing from the bar writes the place, under the episode's id. A seek
  // writes it too, so the writes are counted from after the seek's has landed:
  // otherwise that one would answer for a pause that wrote nothing.
  const placed = (from: number) =>
    state.bookmarkWrites
      .slice(from)
      .some(
        (write) =>
          write.id === "episode-1" &&
          typeof write.body === "object" &&
          (write.body as { position_ms: number }).position_ms >= 12_000,
      );
  await expect.poll(() => placed(0)).toBe(true);
  const beforePause = state.bookmarkWrites.length;
  await bar.getByRole("button", { name: "Pause" }).click();
  await expect
    .poll(async () => (await episodeElement(page, "ticket-episode-1"))?.paused)
    .toBe(true);
  await expect.poll(() => placed(beforePause)).toBe(true);

  // Taken out of the bar, then started again from its page: it resumes where
  // the bookmark says, which the page now shows too.
  await bar.getByRole("button", { name: "Stop and close" }).click();
  await expect(bar).toHaveCount(0);
  await navigateWithin(page, "Podcasts");
  await page.getByRole("button", { name: "Episodes" }).click();
  await expect(first).toContainText("resumes at 0:1");
  await first.getByRole("button", { name: "Play First night" }).click();
  await expect
    .poll(async () => (await episodeElement(page, "ticket-episode-1"))?.paused)
    .toBe(false);
  expect(
    (await episodeElement(page, "ticket-episode-1"))?.currentTime,
  ).toBeGreaterThanOrEqual(12);
});

/**
 * A channel's cover is the hash of the copy the server fetched, and the page
 * reads it through the artwork route like any cover: beside the channel,
 * above its episodes, and in the bar and the system's controls while one of
 * them plays. A channel without one keeps the podcast symbol.
 */
test("shows a channel's cover in its list, above its episodes and in the bar", async ({
  page,
}) => {
  const cover = "c".repeat(64);
  let served = 0;
  await page.route(`**/api/v2/artwork/${cover}`, (route) => {
    served += 1;
    return route.fulfill({ contentType: "image/png", body: PIXEL });
  });
  await recordPlayedElements(page);
  state.channels = [
    {
      ...channelOf("channel-1", "Night Shift", [
        { ...episode("episode-1", "First night", ready), artwork_hash: cover },
      ]),
      artwork_hash: cover,
    },
    channelOf("channel-2", "Day Shift", []),
  ];
  await page.goto("/podcasts");
  const night = page
    .locator(".podcast-channels li")
    .filter({ hasText: "Night Shift" });
  const day = page
    .locator(".podcast-channels li")
    .filter({ hasText: "Day Shift" });
  await expect(night.locator("img.resource-cover")).toBeVisible();
  await expect(day.locator(".resource-cover.cover-fallback svg")).toBeVisible();

  await night.getByRole("button", { name: "Episodes" }).click();
  await expect(
    page.locator(".podcast-episodes img.podcast-cover"),
  ).toBeVisible();
  const first = page
    .locator(".podcast-episodes li")
    .filter({ hasText: "First night" });
  await first.getByRole("button", { name: "Play First night" }).click();

  const bar = page.locator(".player");
  await expect(bar.locator(".player-track strong")).toHaveText("First night");
  await expect(bar.locator("img.player-cover")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () => navigator.mediaSession?.metadata?.artwork?.[0]?.src ?? "",
      ),
    )
    .toMatch(/^blob:/);
  // One request for the hash, however many places show it.
  expect(served).toBe(1);
  await expectNoWcagViolations(page);
});

test("keeps an episode playing when another channel is opened", async ({
  page,
}) => {
  await recordPlayedElements(page);
  state.channels = [
    channelOf("channel-1", "Night Shift", [
      episode("episode-1", "First night", ready),
    ]),
    channelOf("channel-2", "Day Shift", [
      { ...episode("episode-9", "First day", ready), channel_id: "channel-2" },
    ]),
  ];
  await page.goto("/podcasts");
  const night = page
    .locator(".podcast-channels li")
    .filter({ hasText: "Night Shift" });
  await night.getByRole("button", { name: "Episodes" }).click();
  const first = page
    .locator(".podcast-episodes li")
    .filter({ hasText: "First night" });
  await first.getByRole("button", { name: "Play First night" }).click();
  await expect(
    first.getByRole("button", { name: "Pause First night" }),
  ).toBeVisible();

  // Another channel's list is only a list: the episode is the player's, and
  // the bar goes on showing it.
  const day = page
    .locator(".podcast-channels li")
    .filter({ hasText: "Day Shift" });
  await day.getByRole("button", { name: "Episodes" }).click();
  await expect(page.getByRole("heading", { name: "Day Shift" })).toBeVisible();
  await expect(page.locator(".player .player-track strong")).toHaveText(
    "First night",
  );
  await expect
    .poll(async () => (await episodeElement(page, "ticket-episode-1"))?.paused)
    .toBe(false);
});

/**
 * The bookmark writes go out one after the other.
 *
 * A seek writes the place; the end of the episode, moments later, forgets it.
 * Sent together, the two can be served in either order, and the `PUT` landing
 * last brought back the bookmark of an episode heard to the end — the next
 * play resumed it at its last second. Here the seek's `PUT` is held in flight
 * across the end of the episode, then let go.
 */
test("forgets an episode heard to the end even when a write before it was slow", async ({
  page,
}) => {
  await recordPlayedElements(page);
  state.channels = [
    channelOf("channel-1", "Night Shift", [
      episode("episode-1", "First night", { ...ready, duration_ms: 30_000 }),
    ]),
  ];
  await page.goto("/podcasts");
  await page.getByRole("button", { name: "Episodes" }).click();
  await page.getByRole("button", { name: "Play First night" }).click();
  await expect
    .poll(async () => (await episodeElement(page, "ticket-episode-1"))?.paused)
    .toBe(false);

  let release = () => {};
  state.bookmarkGate = new Promise<void>((resolve) => {
    release = resolve;
  });
  // A second before the end: the seek's write is sent, and held.
  const seek = page.locator(".player").getByRole("slider", { name: "Seek" });
  await seek.fill("29");
  await seek.dispatchEvent("mouseup");
  await expect.poll(() => state.bookmarkWrites.length).toBe(1);
  // The episode ends while that write is still out.
  await expect
    .poll(async () => (await episodeElement(page, "ticket-episode-1"))?.paused)
    .toBe(true);
  await expect(
    page.locator(".player").getByRole("button", { name: "Play" }),
  ).toBeVisible();

  release();
  // Both written, the forgetting last: what the server keeps is no place.
  await expect
    .poll(() =>
      state.bookmarkWrites.map((write) =>
        typeof write.body === "string" ? write.body : "PUT",
      ),
    )
    .toEqual(["PUT", "DELETE"]);
  await expect
    .poll(() => state.channels[0].episodes[0].position_ms, { timeout: 3_000 })
    .toBeNull();
  // And it stays so once the held write has been applied as well.
  await page.waitForTimeout(500);
  expect(state.channels[0].episodes[0].position_ms).toBeNull();
});

/**
 * Signing out while an episode's ticket is on its way plays nothing when the
 * ticket lands. The player is gone with the session; the episode would
 * otherwise start on its element, with no bar left to stop it.
 */
test("plays no episode whose ticket lands after the listener signed out", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "signs out from the sidebar");
  await recordPlayedElements(page);
  state.channels = [
    channelOf("channel-1", "Night Shift", [
      episode("episode-1", "First night", ready),
    ]),
  ];
  let release = () => {};
  state.ticketGate = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.goto("/podcasts");
  await page.getByRole("button", { name: "Episodes" }).click();
  const asked = page.waitForRequest(
    (request) =>
      request.url().includes("/podcast-episodes/") ||
      request.url().includes("/stream-ticket"),
  );
  await page.getByRole("button", { name: "Play First night" }).click();
  await asked;

  // Dispatched rather than clicked: the sidebar's footer sits under the
  // fixed player bar at this viewport.
  await page.getByRole("button", { name: "Sign out" }).dispatchEvent("click");
  await expect(page).toHaveURL(/\/login$/);

  const answered = page.waitForResponse((response) =>
    response.url().includes("/stream-ticket"),
  );
  release();
  await answered;
  // Nothing to wait for if the fix holds — so a fixed pause, long enough for
  // the continuation to set a source and start it if it were going to.
  await page.waitForTimeout(750);
  const heard = await episodeElement(page, "ticket-episode-1");
  expect(heard === null || heard.paused).toBe(true);
});

test("offers a listener no podcast control it may not use", async ({
  page,
}) => {
  state.canManage = false;
  state.channels = [
    {
      id: "channel-1",
      // A reader is shown no feed address: a private feed carries its token.
      url: null,
      title: "Night Shift",
      description: null,
      site_url: null,
      image_url: null,
      artwork_hash: null,
      status: "error",
      error: "http_500",
      created_at: 1,
      refreshed_at: 1,
      episode_count: 1,
      episodes: [episode("episode-1", "First night")],
    },
  ];
  await page.goto("/podcasts");
  await expect(page.getByText("last fetch failed: http_500")).toBeVisible();
  await expect(page.getByLabel("Feed address")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Refresh every feed" }),
  ).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Unsubscribe" })).toHaveCount(
    0,
  );
  await page.getByRole("button", { name: "Episodes" }).click();
  const first = page
    .locator(".podcast-episodes li")
    .filter({ hasText: "First night" });
  await expect(first).toContainText("not downloaded");
  await expect(first.getByRole("button", { name: "Download" })).toHaveCount(0);
});
