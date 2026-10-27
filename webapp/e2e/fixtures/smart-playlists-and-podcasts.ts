// The smart playlist and podcast specs' mock `/api/v2`, and what it keeps.

import type { Page } from "@playwright/test";

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

export const LIBRARY_ID = "2740f393-e84e-488a-8a67-745deb23399a";

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
    (condition) =>
      condition.field === "title" && condition.operator === "contains",
  );
  const needle = String(contains?.value ?? "").toLowerCase();
  return catalogue.filter((track) =>
    track.title.toLowerCase().includes(needle),
  );
}

export type Episode = {
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

export const episode = (
  id: string,
  title: string,
  extra: Partial<Episode> = {},
): Episode => ({
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

/**
 * What the mock keeps between requests, which the tests read, replace and
 * reset. Fields of one object rather than module-level `let`s, because a
 * binding imported from another module cannot be reassigned.
 */
class SmartAndPodcastState {
  smart: Array<Record<string, unknown> & { id: string; rules: Rules }> = [];
  smartWrites: Array<{ method: string; body: unknown }> = [];

  canManage = true;
  channels: Array<
    Record<string, unknown> & { id: string; episodes: Episode[] }
  > = [];
  podcastWrites: string[] = [];
  bookmarkWrites: Array<{ id: string; body: unknown }> = [];
  /** Held open by the tests that need a request in flight; instant otherwise. */
  bookmarkGate: Promise<void> = Promise.resolve();
  ticketGate: Promise<void> = Promise.resolve();
}

/** The one instance, shared by the mock and every test in a worker. */
export const state = new SmartAndPodcastState();

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

function listed(playlist: (typeof state.smart)[number]) {
  const { songs: _songs, ...rest } = playlist;
  return rest;
}

async function mockApi(page: Page) {
  await page.context().addCookies([
    {
      name: "waveflow-csrf",
      value: "e2e-csrf",
      url: "http://127.0.0.1:4173",
    },
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
        json: {
          song_count: songs.length,
          duration_ms: songs.length * 200_000,
          songs,
        },
      });
      return;
    }
    if (path === "/api/v2/smart-playlists") {
      if (method === "POST") {
        const body = request.postDataJSON();
        state.smartWrites.push({ method, body });
        const songs = evaluate(body.rules);
        const created = {
          id: `smart-${state.smart.length + 1}`,
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
        state.smart = [...state.smart, created];
        await route.fulfill({ status: 201, json: created });
        return;
      }
      await route.fulfill({ json: state.smart.map(listed) });
      return;
    }
    if (path.startsWith("/api/v2/smart-playlists/")) {
      const id = path.slice("/api/v2/smart-playlists/".length);
      const found = state.smart.find((playlist) => playlist.id === id);
      if (!found) {
        await route.fulfill({ status: 404, json: { code: "not_found" } });
        return;
      }
      if (method === "DELETE") {
        state.smartWrites.push({ method, body: id });
        state.smart = state.smart.filter((playlist) => playlist.id !== id);
        await route.fulfill({ status: 204, body: "" });
        return;
      }
      if (method === "PATCH") {
        const body = request.postDataJSON();
        state.smartWrites.push({ method, body });
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
        state.podcastWrites.push(`subscribe ${body.url}`);
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
        state.channels = [channel];
        await route.fulfill({ status: 201, json: channel });
        return;
      }
      await route.fulfill({
        json: {
          enabled: true,
          can_manage: state.canManage,
          channels: state.channels.map(
            ({ episodes: _episodes, ...channel }) => channel,
          ),
        },
      });
      return;
    }
    if (path === "/api/v2/podcasts/refresh") {
      state.podcastWrites.push("refresh");
      await route.fulfill({ status: 202, body: "" });
      return;
    }
    if (path.startsWith("/api/v2/podcasts/")) {
      const id = path.slice("/api/v2/podcasts/".length);
      if (method === "DELETE") {
        state.podcastWrites.push(`unsubscribe ${id}`);
        state.channels = state.channels.filter((channel) => channel.id !== id);
        await route.fulfill({ status: 204, body: "" });
        return;
      }
      const channel = state.channels.find((candidate) => candidate.id === id);
      await route.fulfill(
        channel
          ? { json: channel }
          : { status: 404, json: { code: "not_found" } },
      );
      return;
    }
    const download = path.match(
      /^\/api\/v2\/podcast-episodes\/([^/]+)\/download$/,
    );
    if (download) {
      state.podcastWrites.push(`download ${download[1]}`);
      for (const channel of state.channels) {
        for (const item of channel.episodes) {
          if (item.id === download[1]) {
            Object.assign(item, {
              status: "completed",
              suffix: "wav",
              size: 8044,
            });
          }
        }
      }
      await route.fulfill({ status: 202, body: "" });
      return;
    }
    const ticket = path.match(/^\/api\/v2\/tracks\/([^/]+)\/stream-ticket$/);
    if (ticket) {
      await state.ticketGate;
      await route.fulfill({
        json: {
          url: `/api/v2/stream/ticket-${ticket[1]}`,
          expires_at: Date.now() + 3_600_000,
        },
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
      state.bookmarkWrites.push({ id: bookmark[1], body });
      // Held by the ordering test, after the arrival is recorded and before
      // the write is applied: a request still in flight, as a slow one is.
      if (method === "PUT") await state.bookmarkGate;
      // Kept, as the server keeps it: the episode reads back where it was
      // left, so a resume can be told from a start.
      for (const channel of state.channels) {
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

/**
 * Before every test: the mock's state back to its defaults, and `/api/v2`
 * mocked over it. Both specs install it with `test.beforeEach`.
 */
export const resetAndMockSmartAndPodcastApi = async ({
  page,
}: {
  page: Page;
}) => {
  state.smart = [];
  state.smartWrites = [];
  state.canManage = true;
  state.channels = [];
  state.podcastWrites = [];
  state.bookmarkWrites = [];
  state.bookmarkGate = Promise.resolve();
  state.ticketGate = Promise.resolve();
  await mockApi(page);
};
