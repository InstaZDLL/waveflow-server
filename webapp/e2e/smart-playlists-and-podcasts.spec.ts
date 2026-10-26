import AxeBuilder from "@axe-core/playwright";
import { expect, type Page, test } from "@playwright/test";

/**
 * Smart playlists and podcasts (RFC-025), against a mock that keeps what it is
 * sent: an editor that saved nothing, or a download that changed nothing,
 * would otherwise pass against a fixed answer.
 *
 * Shaped from the server's own types — `SmartPlaylistItem`,
 * `PodcastOverview`, `PodcastChannelItem` — not from the client's
 * declarations of them.
 */

const session = {
  access_token: "e2e-access-token",
  user: { id: "user-1", username: "listener", role: "admin" },
  device_id: "device-1",
};

const LIBRARY_ID = "2740f393-e84e-488a-8a67-745deb23399a";

const song = (index: number, title: string) => ({
  id: `song-${index}`,
  library_id: "library-1",
  album_id: "album-1",
  title,
  album: "Post",
  artist: "Björk",
  artist_id: "artist-1",
  artwork_hash: null,
  duration_ms: 200_000,
  track: index,
  disc: 1,
  starred_at: null,
  user_rating: null,
  codec: "FLAC",
  suffix: "flac",
  size: 1000,
  bitrate: 900,
  sample_rate: 44_100,
  channels: 2,
  bit_depth: 16,
});

const catalogue = [
  song(1, "Army of Me"),
  song(2, "Hyperballad"),
  song(3, "Isobel"),
];

type Rules = {
  match: "all" | "any";
  conditions: Array<{ field: string; operator: string; value: unknown }>;
  sort: string | null;
  order: "asc" | "desc";
  limit: number | null;
};

/** What the mock answers for rules: the one condition it understands. */
function evaluate(rules: Rules) {
  const contains = rules.conditions.find(
    (condition) => condition.field === "title" && condition.operator === "contains",
  );
  const needle = String(contains?.value ?? "").toLowerCase();
  return catalogue.filter((track) => track.title.toLowerCase().includes(needle));
}

let smart: Array<Record<string, unknown> & { id: string; rules: Rules }> = [];
let smartWrites: Array<{ method: string; body: unknown }> = [];

type Episode = {
  id: string;
  channel_id: string;
  channel_title: string;
  artwork_hash: string | null;
  title: string;
  description: string | null;
  published_at: number | null;
  duration_ms: number | null;
  status: "new" | "downloading" | "completed" | "error" | "deleted";
  error: string | null;
  size: number | null;
  suffix: string | null;
  content_type: string | null;
  position_ms: number | null;
  created_at: number;
};

let canManage = true;
let channels: Array<Record<string, unknown> & { id: string; episodes: Episode[] }> = [];
let podcastWrites: string[] = [];
let bookmarkWrites: Array<{ id: string; body: unknown }> = [];
/** Held open by the tests that need a request in flight; instant otherwise. */
let bookmarkGate: Promise<void> = Promise.resolve();
let ticketGate: Promise<void> = Promise.resolve();

const episode = (id: string, title: string, extra: Partial<Episode> = {}): Episode => ({
  id,
  channel_id: "channel-1",
  channel_title: "Night Shift",
  artwork_hash: null,
  title,
  description: null,
  published_at: 1_704_448_800_000,
  duration_ms: 720_000,
  status: "new",
  error: null,
  size: null,
  suffix: null,
  content_type: null,
  position_ms: null,
  created_at: 1_704_448_800_000,
  ...extra,
});

/** One second of silence, as a WAV the browser can play. */
function silence(): Buffer {
  const samples = 8000 * 30;
  const header = Buffer.alloc(44);
  header.write("RIFF", 0);
  header.writeUInt32LE(36 + samples, 4);
  header.write("WAVE", 8);
  header.write("fmt ", 12);
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(1, 22);
  header.writeUInt32LE(8000, 24);
  header.writeUInt32LE(8000, 28);
  header.writeUInt16LE(1, 32);
  header.writeUInt16LE(8, 34);
  header.write("data", 36);
  header.writeUInt32LE(samples, 40);
  return Buffer.concat([header, Buffer.alloc(samples, 128)]);
}

async function expectNoWcagViolations(page: Page) {
  await page.evaluate(() =>
    Promise.all(
      document
        .getAnimations()
        .filter(
          (animation) =>
            animation.effect?.getComputedTiming().endTime !== Infinity,
        )
        .map((animation) => animation.finished.catch(() => undefined)),
    ),
  );
  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  expect(results.violations).toEqual([]);
}

function listed(playlist: (typeof smart)[number]) {
  const { songs: _songs, ...rest } = playlist;
  return rest;
}

async function mockApi(page: Page) {
  await page.context().addCookies([
    { name: "waveflow-csrf", value: "e2e-csrf", url: "http://127.0.0.1:4173" },
  ]);
  await page.route("**/api/v2/**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const method = request.method();
    const path = url.pathname;
    if (path === "/api/v2/web/auth/refresh") {
      await route.fulfill({ json: session });
      return;
    }
    if (path === "/api/v2/playlists") {
      await route.fulfill({ json: [] });
      return;
    }
    if (path === "/api/v2/libraries") {
      // The whole `Library` shape, with a real UUID: the editor sends the id.
      await route.fulfill({
        json: [
          {
            id: LIBRARY_ID,
            name: "Ma musique",
            visibility: "private",
            role: "owner",
            last_scan_started_at: null,
            last_scan_completed_at: null,
            accepts_uploads: false,
            accepts_canvas: false,
            follow_links: false,
          },
        ],
      });
      return;
    }
    if (path === "/api/v2/smart-playlists/preview") {
      const songs = evaluate(request.postDataJSON().rules);
      await route.fulfill({
        json: { song_count: songs.length, duration_ms: songs.length * 200_000, songs },
      });
      return;
    }
    if (path === "/api/v2/smart-playlists") {
      if (method === "POST") {
        const body = request.postDataJSON();
        smartWrites.push({ method, body });
        const songs = evaluate(body.rules);
        const created = {
          id: `smart-${smart.length + 1}`,
          owner_id: "user-1",
          owner_username: "listener",
          name: body.name,
          comment: null,
          public: Boolean(body.public),
          rules: body.rules,
          created_at: 1,
          updated_at: 1,
          evaluated_at: 1,
          song_count: songs.length,
          duration_ms: songs.length * 200_000,
          songs,
        };
        smart = [...smart, created];
        await route.fulfill({ status: 201, json: created });
        return;
      }
      await route.fulfill({ json: smart.map(listed) });
      return;
    }
    if (path.startsWith("/api/v2/smart-playlists/")) {
      const id = path.slice("/api/v2/smart-playlists/".length);
      const found = smart.find((playlist) => playlist.id === id);
      if (!found) {
        await route.fulfill({ status: 404, json: { code: "not_found" } });
        return;
      }
      if (method === "DELETE") {
        smartWrites.push({ method, body: id });
        smart = smart.filter((playlist) => playlist.id !== id);
        await route.fulfill({ status: 204, body: "" });
        return;
      }
      if (method === "PATCH") {
        const body = request.postDataJSON();
        smartWrites.push({ method, body });
        Object.assign(found, body);
        await route.fulfill({ json: found });
        return;
      }
      // Opening one evaluates it, and the listings after it report what this
      // evaluation found — the server's own memory of the last answer.
      const songs = evaluate(found.rules);
      Object.assign(found, {
        song_count: songs.length,
        duration_ms: songs.length * 200_000,
        evaluated_at: 2,
      });
      await route.fulfill({ json: { ...found, songs } });
      return;
    }
    if (path === "/api/v2/podcasts") {
      if (method === "POST") {
        const body = request.postDataJSON();
        podcastWrites.push(`subscribe ${body.url}`);
        const channel = {
          id: "channel-1",
          url: body.url,
          title: "Night Shift",
          description: "Talk after dark",
          site_url: null,
          image_url: null,
          artwork_hash: null,
          status: "completed",
          error: null,
          created_at: 1,
          refreshed_at: 1,
          episode_count: 2,
          episodes: [
            episode("episode-1", "First night"),
            episode("episode-2", "Second night", {
              status: "completed",
              suffix: "wav",
              size: 8044,
              position_ms: 30_000,
            }),
          ],
        };
        channels = [channel];
        await route.fulfill({ status: 201, json: channel });
        return;
      }
      await route.fulfill({
        json: {
          enabled: true,
          can_manage: canManage,
          channels: channels.map(({ episodes: _episodes, ...channel }) => channel),
        },
      });
      return;
    }
    if (path === "/api/v2/podcasts/refresh") {
      podcastWrites.push("refresh");
      await route.fulfill({ status: 202, body: "" });
      return;
    }
    if (path.startsWith("/api/v2/podcasts/")) {
      const id = path.slice("/api/v2/podcasts/".length);
      if (method === "DELETE") {
        podcastWrites.push(`unsubscribe ${id}`);
        channels = channels.filter((channel) => channel.id !== id);
        await route.fulfill({ status: 204, body: "" });
        return;
      }
      const channel = channels.find((candidate) => candidate.id === id);
      await route.fulfill(
        channel ? { json: channel } : { status: 404, json: { code: "not_found" } },
      );
      return;
    }
    const download = path.match(/^\/api\/v2\/podcast-episodes\/([^/]+)\/download$/);
    if (download) {
      podcastWrites.push(`download ${download[1]}`);
      for (const channel of channels) {
        for (const item of channel.episodes) {
          if (item.id === download[1]) {
            Object.assign(item, { status: "completed", suffix: "wav", size: 8044 });
          }
        }
      }
      await route.fulfill({ status: 202, body: "" });
      return;
    }
    const ticket = path.match(/^\/api\/v2\/tracks\/([^/]+)\/stream-ticket$/);
    if (ticket) {
      await ticketGate;
      await route.fulfill({
        json: { url: `/api/v2/stream/ticket-${ticket[1]}`, expires_at: Date.now() + 3_600_000 },
      });
      return;
    }
    if (path.startsWith("/api/v2/stream/")) {
      await route.fulfill({
        status: 200,
        headers: { "content-type": "audio/wav", "accept-ranges": "bytes" },
        body: silence(),
      });
      return;
    }
    const bookmark = path.match(/^\/api\/v2\/bookmarks\/([^/]+)$/);
    if (bookmark) {
      const body = method === "PUT" ? request.postDataJSON() : method;
      bookmarkWrites.push({ id: bookmark[1], body });
      // Held by the ordering test, after the arrival is recorded and before
      // the write is applied: a request still in flight, as a slow one is.
      if (method === "PUT") await bookmarkGate;
      // Kept, as the server keeps it: the episode reads back where it was
      // left, so a resume can be told from a start.
      for (const channel of channels) {
        for (const item of channel.episodes) {
          if (item.id === bookmark[1]) {
            item.position_ms = method === "PUT" ? body.position_ms : null;
          }
        }
      }
      await route.fulfill({ status: 204, body: "" });
      return;
    }
    await route.fulfill({ status: 404, json: { error: "not found" } });
  });
}

test.beforeEach(async ({ page }) => {
  smart = [];
  smartWrites = [];
  canManage = true;
  channels = [];
  podcastWrites = [];
  bookmarkWrites = [];
  bookmarkGate = Promise.resolve();
  ticketGate = Promise.resolve();
  await mockApi(page);
});

test("writes a smart playlist's rules, previews them and keeps them", async ({
  page,
}) => {
  await page.goto("/playlists");
  const section = page.locator(".smart-playlists");
  await expect(section.getByRole("heading", { name: "Smart playlists" })).toBeVisible();
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
  expect(smartWrites).toHaveLength(1);
  expect(smartWrites[0].body).toEqual({
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
  const card = section.locator(".smart-playlist").filter({ hasText: "With a y" });
  await expect(card).toContainText("2 tracks");

  // Edited: the stored rules come back into the form, and a save replaces
  // them whole.
  await card.getByRole("button", { name: "Edit rules" }).click();
  const again = page.getByRole("form", { name: "Edit rules" });
  await expect(again.getByLabel("Value of condition 1")).toHaveValue("y");
  await expect(again.getByLabel("Upper bound of condition 2")).toHaveValue("1999");
  await again.getByRole("button", { name: "Remove condition 2" }).click();
  await again.getByRole("button", { name: "Save smart playlist" }).click();
  await expect(again).toHaveCount(0);
  expect(smartWrites[1]).toMatchObject({
    method: "PATCH",
    body: { rules: { conditions: [{ field: "title", operator: "contains", value: "y" }] } },
  });

  await card.getByRole("button", { name: "Delete" }).click();
  await expect(card).toHaveCount(0);
  expect(smartWrites[2]).toEqual({ method: "DELETE", body: "smart-1" });
});

test("lists another account's smart playlist uncounted until it is opened", async ({
  page,
}) => {
  smart = [
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
  const card = page.locator(".smart-playlist").filter({ hasText: "Their y songs" });
  await expect(card).toContainText("Counted when opened");
  await expect(card).toContainText("By guest");
  await expect(card.getByRole("button", { name: "Edit rules" })).toHaveCount(0);
  await expect(card.getByRole("button", { name: "Delete" })).toHaveCount(0);
  await card.getByRole("button", { name: "Play" }).click();
  await expect(card).toContainText("2 tracks");
  await expect(page.locator(".player")).toContainText("Army of Me");
});

/** A smart playlist of this account's, as the listing answers it. */
const ownSmart = (id: string, name: string, needle: string, counted: number | null) => ({
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

test("plays a smart playlist whose remembered count is zero", async ({ page }) => {
  // Counted at zero when last opened; matching tracks arrived since. A count
  // is only what was true then, so it must not keep the playlist from playing.
  smart = [ownSmart("smart-stale", "Was empty", "y", 0)];
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
  smart = [
    ownSmart("smart-a", "Playlist A", "y", 2),
    ownSmart("smart-b", "Playlist B", "Iso", 1),
  ];
  await page.goto("/playlists");
  const first = page.locator(".smart-playlist").filter({ hasText: "Playlist A" });
  await first.getByRole("button", { name: "Edit rules" }).click();
  const editor = page.getByRole("form", { name: "Edit rules" });
  await editor.getByRole("button", { name: "Preview" }).click();
  await expect(editor.getByRole("status")).toContainText("2 tracks match");

  // Another playlist: its own rules, and no preview of the first's.
  const second = page.locator(".smart-playlist").filter({ hasText: "Playlist B" });
  await second.getByRole("button", { name: "Edit rules" }).click();
  await expect(editor.getByLabel("Name", { exact: true })).toHaveValue("Playlist B");
  await expect(editor.getByLabel("Value of condition 1")).toHaveValue("Iso");
  await expect(editor.getByRole("status")).toHaveCount(0);

  // A preview, then a change to the rules: the preview goes.
  await editor.getByRole("button", { name: "Preview" }).click();
  await expect(editor.getByRole("status")).toContainText("1 track matches");
  await editor.getByLabel("Value of condition 1").fill("Army");
  await expect(editor.getByRole("status")).toHaveCount(0);
});

test("chooses a library for a condition, by name, and sends its id", async ({ page }) => {
  await page.goto("/playlists");
  const section = page.locator(".smart-playlists");
  await section.getByRole("button", { name: "New smart playlist" }).click();
  const editor = page.getByRole("form", { name: "New smart playlist" });
  await editor.getByLabel("Name", { exact: true }).fill("From one library");
  await editor.getByLabel("Field of condition 1").selectOption("library");
  await expect(
    editor.getByLabel("Comparison of condition 1").locator("option"),
  ).toHaveText(["is", "is not"]);
  await expect(editor.getByRole("button", { name: "Save smart playlist" })).toBeDisabled();
  await editor.getByLabel("Value of condition 1").selectOption({ label: "Ma musique" });
  await editor.getByRole("button", { name: "Save smart playlist" }).click();
  await expect(editor).toHaveCount(0);
  expect(smartWrites[0].body).toMatchObject({
    rules: {
      conditions: [{ field: "library", operator: "is", value: LIBRARY_ID }],
    },
  });
});

test("subscribes to a podcast, downloads, plays and keeps the place", async ({
  page,
}) => {
  await page.goto("/podcasts");
  await expect(page.getByRole("heading", { name: "Podcasts" })).toBeVisible();
  await expect(page.getByText("No podcasts yet.")).toBeVisible();
  await page.getByLabel("Feed address").fill("https://feeds.example.org/night.xml");
  await page.getByRole("button", { name: "Subscribe" }).click();
  expect(podcastWrites).toEqual(["subscribe https://feeds.example.org/night.xml"]);

  const episodes = page.locator(".podcast-episodes");
  await expect(episodes.getByRole("heading", { name: "Night Shift" })).toBeVisible();
  const first = episodes.locator("li").filter({ hasText: "First night" });
  await expect(first).toContainText("not downloaded");
  await first.getByRole("button", { name: "Download" }).click();
  expect(podcastWrites).toContain("download episode-1");
  await expect(first).toContainText("ready");

  // The saved place is shown, and playing resumes from it.
  const second = episodes.locator("li").filter({ hasText: "Second night" });
  await expect(second).toContainText("resumes at 0:30");
  await expectNoWcagViolations(page);
  await first.getByRole("button", { name: "Play First night" }).click();
  await expect(first.getByRole("button", { name: "Pause First night" })).toBeVisible();
  await first.getByRole("button", { name: "Pause First night" }).click();
  // Pausing writes the place through the bookmark route, under the episode's id.
  await expect.poll(() => bookmarkWrites.map((write) => write.id)).toContain("episode-1");

  await page.getByRole("button", { name: "Refresh every feed" }).click();
  expect(podcastWrites).toContain("refresh");
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

/**
 * Moves within the client, as a click on its navigation does — never by
 * loading a page, which would start the player over and prove nothing about
 * what survives a navigation.
 */
async function navigateWithin(page: Page, name: string) {
  const sidebar = page.locator(".sidebar");
  if (await sidebar.isVisible()) {
    await sidebar.getByRole("link", { name, exact: true }).click();
    return;
  }
  const bar = page.locator(".mobile-navigation");
  const direct = bar.getByRole("link", { name, exact: true });
  if (await direct.count()) {
    await direct.click();
    return;
  }
  await bar.getByRole("button", { name: "More" }).click();
  await page.getByRole("dialog").getByRole("link", { name, exact: true }).click();
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
  channels = [
    channelOf("channel-1", "Night Shift", [
      // Thirty seconds of sound, as the route serves it: the duration the
      // feed announced is replaced by the file's once it loads.
      episode("episode-1", "First night", { ...ready, duration_ms: 30_000 }),
    ]),
  ];
  await page.goto("/podcasts");
  await page.getByRole("button", { name: "Episodes" }).click();
  const first = page.locator(".podcast-episodes li").filter({ hasText: "First night" });
  await first.getByRole("button", { name: "Play First night" }).click();

  const bar = page.locator(".player");
  await expect(bar.locator(".player-track strong")).toHaveText("First night");
  await expect(bar.locator(".player-track")).toContainText("Night Shift");
  await expect(bar.getByRole("button", { name: "Pause" })).toBeVisible();
  await expect.poll(async () => (await episodeElement(page, "ticket-episode-1"))?.paused).toBe(false);

  // Away from the page that started it: still playing, still in the bar.
  await navigateWithin(page, "Playlists");
  await expect(page.locator(".smart-playlists")).toBeVisible();
  await expect(bar.locator(".player-track strong")).toHaveText("First night");
  await expect
    .poll(async () => (await episodeElement(page, "ticket-episode-1"))?.currentTime ?? 0)
    .toBeGreaterThan(0.2);
  expect((await episodeElement(page, "ticket-episode-1"))?.paused).toBe(false);
  // An episode has a line and a scrubber, and nothing the queue has: no next,
  // no previous, no shuffle, no repeat.
  const seek = bar.getByRole("slider", { name: "Seek" });
  await expect(seek).toBeVisible();  await expect(bar.getByRole("button", { name: "Next track" })).toHaveCount(0);
  await expect(bar.getByRole("button", { name: "Previous track" })).toHaveCount(0);
  await expect(bar.getByRole("button", { name: "Shuffle" })).toHaveCount(0);
  await expectNoWcagViolations(page);

  // Seeking from the bar moves the episode, not a track.
  await seek.fill("12");
  await seek.dispatchEvent("mouseup");
  await expect
    .poll(async () => (await episodeElement(page, "ticket-episode-1"))?.currentTime ?? 0)
    .toBeGreaterThanOrEqual(12);

  // Pausing from the bar writes the place, under the episode's id. A seek
  // writes it too, so the writes are counted from after the seek's has landed:
  // otherwise that one would answer for a pause that wrote nothing.
  const placed = (from: number) =>
    bookmarkWrites
      .slice(from)
      .some(
        (write) =>
          write.id === "episode-1" &&
          typeof write.body === "object" &&
          (write.body as { position_ms: number }).position_ms >= 12_000,
      );
  await expect.poll(() => placed(0)).toBe(true);
  const beforePause = bookmarkWrites.length;
  await bar.getByRole("button", { name: "Pause" }).click();
  await expect.poll(async () => (await episodeElement(page, "ticket-episode-1"))?.paused).toBe(true);
  await expect.poll(() => placed(beforePause)).toBe(true);

  // Taken out of the bar, then started again from its page: it resumes where
  // the bookmark says, which the page now shows too.
  await bar.getByRole("button", { name: "Stop and close" }).click();
  await expect(bar).toHaveCount(0);
  await navigateWithin(page, "Podcasts");
  await page.getByRole("button", { name: "Episodes" }).click();
  await expect(first).toContainText("resumes at 0:1");
  await first.getByRole("button", { name: "Play First night" }).click();
  await expect.poll(async () => (await episodeElement(page, "ticket-episode-1"))?.paused).toBe(false);
  expect((await episodeElement(page, "ticket-episode-1"))?.currentTime).toBeGreaterThanOrEqual(12);
});

/** A one-pixel PNG, for the covers the artwork route is asked for. */
const PIXEL = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
  "base64",
);

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
  channels = [
    {
      ...channelOf("channel-1", "Night Shift", [
        { ...episode("episode-1", "First night", ready), artwork_hash: cover },
      ]),
      artwork_hash: cover,
    },
    channelOf("channel-2", "Day Shift", []),
  ];
  await page.goto("/podcasts");
  const night = page.locator(".podcast-channels li").filter({ hasText: "Night Shift" });
  const day = page.locator(".podcast-channels li").filter({ hasText: "Day Shift" });
  await expect(night.locator("img.resource-cover")).toBeVisible();
  await expect(day.locator(".resource-cover.cover-fallback svg")).toBeVisible();

  await night.getByRole("button", { name: "Episodes" }).click();
  await expect(page.locator(".podcast-episodes img.podcast-cover")).toBeVisible();
  const first = page.locator(".podcast-episodes li").filter({ hasText: "First night" });
  await first.getByRole("button", { name: "Play First night" }).click();

  const bar = page.locator(".player");
  await expect(bar.locator(".player-track strong")).toHaveText("First night");
  await expect(bar.locator("img.player-cover")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() => navigator.mediaSession?.metadata?.artwork?.[0]?.src ?? ""),
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
  channels = [
    channelOf("channel-1", "Night Shift", [episode("episode-1", "First night", ready)]),
    channelOf("channel-2", "Day Shift", [
      { ...episode("episode-9", "First day", ready), channel_id: "channel-2" },
    ]),
  ];
  await page.goto("/podcasts");
  const night = page.locator(".podcast-channels li").filter({ hasText: "Night Shift" });
  await night.getByRole("button", { name: "Episodes" }).click();
  const first = page.locator(".podcast-episodes li").filter({ hasText: "First night" });
  await first.getByRole("button", { name: "Play First night" }).click();
  await expect(first.getByRole("button", { name: "Pause First night" })).toBeVisible();

  // Another channel's list is only a list: the episode is the player's, and
  // the bar goes on showing it.
  const day = page.locator(".podcast-channels li").filter({ hasText: "Day Shift" });
  await day.getByRole("button", { name: "Episodes" }).click();
  await expect(page.getByRole("heading", { name: "Day Shift" })).toBeVisible();
  await expect(page.locator(".player .player-track strong")).toHaveText("First night");
  await expect.poll(async () => (await episodeElement(page, "ticket-episode-1"))?.paused).toBe(false);
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
  channels = [
    channelOf("channel-1", "Night Shift", [
      episode("episode-1", "First night", { ...ready, duration_ms: 30_000 }),
    ]),
  ];
  await page.goto("/podcasts");
  await page.getByRole("button", { name: "Episodes" }).click();
  await page.getByRole("button", { name: "Play First night" }).click();
  await expect.poll(async () => (await episodeElement(page, "ticket-episode-1"))?.paused).toBe(false);

  let release = () => {};
  bookmarkGate = new Promise<void>((resolve) => {
    release = resolve;
  });
  // A second before the end: the seek's write is sent, and held.
  const seek = page.locator(".player").getByRole("slider", { name: "Seek" });
  await seek.fill("29");
  await seek.dispatchEvent("mouseup");
  await expect.poll(() => bookmarkWrites.length).toBe(1);
  // The episode ends while that write is still out.
  await expect.poll(async () => (await episodeElement(page, "ticket-episode-1"))?.paused).toBe(true);
  await expect(page.locator(".player").getByRole("button", { name: "Play" })).toBeVisible();

  release();
  // Both written, the forgetting last: what the server keeps is no place.
  await expect
    .poll(() => bookmarkWrites.map((write) => (typeof write.body === "string" ? write.body : "PUT")))
    .toEqual(["PUT", "DELETE"]);
  await expect
    .poll(() => channels[0].episodes[0].position_ms, { timeout: 3_000 })
    .toBeNull();
  // And it stays so once the held write has been applied as well.
  await page.waitForTimeout(500);
  expect(channels[0].episodes[0].position_ms).toBeNull();
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
  channels = [
    channelOf("channel-1", "Night Shift", [episode("episode-1", "First night", ready)]),
  ];
  let release = () => {};
  ticketGate = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.goto("/podcasts");
  await page.getByRole("button", { name: "Episodes" }).click();
  const asked = page.waitForRequest((request) =>
    request.url().includes("/podcast-episodes/") || request.url().includes("/stream-ticket"),
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

test("offers a listener no podcast control it may not use", async ({ page }) => {
  canManage = false;
  channels = [
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
  await expect(page.getByRole("button", { name: "Refresh every feed" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Unsubscribe" })).toHaveCount(0);
  await page.getByRole("button", { name: "Episodes" }).click();
  const first = page.locator(".podcast-episodes li").filter({ hasText: "First night" });
  await expect(first).toContainText("not downloaded");
  await expect(first.getByRole("button", { name: "Download" })).toHaveCount(0);
});
