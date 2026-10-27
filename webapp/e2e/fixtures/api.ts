// The mock `/api/v2`, and the hook that resets and installs it per test.

import { type Page, test } from "@playwright/test";

import {
  albumDetail,
  albums,
  artistDetail,
  type Client,
  correctableTrack,
  fileArtists,
  freshClients,
  freshDestinations,
  freshUncertain,
  freshUploadSession,
  genreSongs,
  genres,
  history,
  imageHash,
  importReport,
  library,
  linkState,
  type Offer,
  PURGE_GRACE_MS,
  playlists,
  type Recipient,
  session,
  song,
  statsFor,
  track,
} from "./catalogue";
import { mock } from "./state";

async function mockAuthenticatedApi(page: Page) {
  await page.context().addCookies([
    {
      name: "waveflow-csrf",
      value: "e2e-csrf",
      // The preview's own origin, whichever port it was given.
      url: test.info().project.use.baseURL ?? "http://127.0.0.1:4173",
    },
  ]);
  await page.route("**/api/v2/**", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/api/v2/web/auth/refresh") {
      await route.fulfill({ json: session });
      return;
    }
    if (url.pathname === "/api/v2/albums/album-1") {
      // Held open by the concurrency test; instant for everyone else.
      await mock.slowAlbum;
      await route.fulfill({ json: { ...albums[0], songs: [track] } });
      return;
    }
    if (url.pathname === "/api/v2/scans/scan-1/events" && mock.scanDrops) {
      await route.abort("connectionfailed");
      return;
    }
    if (url.pathname === "/api/v2/scans/scan-1/events" && mock.scanFails) {
      await route.fulfill({ status: 503, body: "" });
      return;
    }
    if (url.pathname === "/api/v2/scans/scan-1/events") {
      // Two frames, deliberately split so the client has to hold a partial
      // one between reads — which is what a real stream does.
      const job = (processed: number, status: string) =>
        JSON.stringify({
          id: "scan-1",
          library_id: "library-1",
          status,
          total_files: 10,
          processed_files: processed,
          added: processed,
          updated: 0,
          moved: 0,
          skipped: 0,
          unavailable: 0,
          errors: 0,
          excluded: 0,
          skipped_links: 0,
          current_path: `/music/track-${processed}.flac`,
          message: null,
        });
      await route.fulfill({
        status: 200,
        headers: { "content-type": "text/event-stream" },
        body:
          `event: snapshot\ndata: ${job(3, "running")}\n\n` +
          `: keep-alive\n\n` +
          `event: progress\ndata: ${job(10, "completed")}\n\n`,
      });
      return;
    }
    if (url.pathname.endsWith("/scans")) {
      await route.fulfill({ json: { scan_id: "scan-1" } });
      return;
    }
    if (url.pathname === "/api/v2/admin/users") {
      await route.fulfill({
        json: [
          {
            id: "user-1",
            username: "listener",
            role: "admin",
            disabled: false,
            has_subsonic_credential: false,
            folder_ids: ["library-1"],
          },
        ],
      });
      return;
    }
    // RFC-021: a library as an administrator manages it, and its missing
    // tracks with their purge.
    if (
      url.pathname.endsWith("/settings") &&
      url.pathname.includes("/libraries/")
    ) {
      const id = url.pathname.split("/")[4];
      const found = mock.libraries.find((entry) => entry.id === id);
      await route.fulfill({
        json: {
          id,
          name: found?.name ?? id,
          path: `/srv/music/${id}`,
          follow_links: found?.follow_links ?? false,
          watch: true,
        },
      });
      return;
    }
    if (url.pathname.endsWith("/unavailable-tracks/purge")) {
      const asked = route.request().postDataJSON() as
        | { all: true }
        | { track_ids: string[] };
      mock.purgeAsks.push(asked);
      // As the server does with its default grace window: out of sight now,
      // restorable for a week.
      const now = Date.now();
      const taken = mock.missingTracks.filter(
        (entry) => "all" in asked || asked.track_ids.includes(entry.id),
      );
      mock.missingTracks = mock.missingTracks.filter(
        (entry) => !taken.includes(entry),
      );
      mock.purgedTracks = [
        ...mock.purgedTracks,
        ...taken.map((entry) => ({
          ...entry,
          purged_at: now,
          deleted_after: now + PURGE_GRACE_MS,
        })),
      ];
      await route.fulfill({
        json: { purged: taken.length, restorable_until: now + PURGE_GRACE_MS },
      });
      return;
    }
    if (url.pathname.endsWith("/purged-tracks/restore")) {
      const asked = route.request().postDataJSON() as
        | { all: true }
        | { track_ids: string[] };
      mock.restoreAsks.push(asked);
      const back = mock.purgedTracks.filter(
        (entry) => "all" in asked || asked.track_ids.includes(entry.id),
      );
      mock.purgedTracks = mock.purgedTracks.filter(
        (entry) => !back.includes(entry),
      );
      mock.missingTracks = [
        ...mock.missingTracks,
        ...back.map((entry) => ({
          ...entry,
          purged_at: null,
          deleted_after: null,
        })),
      ];
      await route.fulfill({ json: { restored: back.length } });
      return;
    }
    if (url.pathname.endsWith("/purged-tracks")) {
      const offset = Number(url.searchParams.get("offset") ?? 0);
      const limit = Number(url.searchParams.get("limit") ?? 100);
      await route.fulfill({
        json: {
          total: mock.purgedTracks.length,
          tracks: mock.purgedTracks.slice(offset, offset + limit),
        },
      });
      return;
    }
    if (url.pathname.endsWith("/unavailable-tracks")) {
      const offset = Number(url.searchParams.get("offset") ?? 0);
      const limit = Number(url.searchParams.get("limit") ?? 100);
      await route.fulfill({
        json: {
          total: mock.missingTracks.length,
          tracks: mock.missingTracks.slice(offset, offset + limit),
        },
      });
      return;
    }
    if (url.pathname.endsWith("/members") && mock.membersFail) {
      await route.fulfill({ status: 500, json: { error: "boom" } });
      return;
    }
    if (url.pathname.endsWith("/members")) {
      await route.fulfill({
        json: [
          {
            user_id: "user-1",
            username: "listener",
            role: "owner",
            created_at: 1,
          },
          {
            user_id: "user-2",
            username: "guest",
            role: "listener",
            created_at: 2,
          },
        ],
      });
      return;
    }
    if (url.pathname === "/api/v2/radio-stations") {
      if (route.request().method() === "POST") {
        mock.radioWrites.push("POST");
        await mock.radioGate;
        const draft = route.request().postDataJSON();
        mock.stationImages.push(draft.image_url);
        const image = draft.image_url ?? null;
        const created = {
          id: `station-${mock.stations.length + 1}`,
          name: String(draft.name).trim(),
          stream_url: draft.stream_url,
          homepage_url: draft.homepage_url ?? null,
          image_url: image,
          artwork_hash: image ? imageHash(image) : null,
          created_at: 1_700_000_000_000,
          updated_at: 1_700_000_000_000,
        };
        mock.stations = [...mock.stations, created].sort((a, b) =>
          a.name.localeCompare(b.name),
        );
        await route.fulfill({ status: 201, json: created });
        return;
      }
      await route.fulfill({ json: mock.stations });
      return;
    }
    if (url.pathname.startsWith("/api/v2/radio-stations/")) {
      const id = url.pathname.slice("/api/v2/radio-stations/".length);
      if (route.request().method() === "DELETE") {
        mock.stations = mock.stations.filter((station) => station.id !== id);
        await route.fulfill({ status: 204, body: "" });
        return;
      }
      // A replacement: every field arrives, so an absent homepage clears it.
      // The image is the exception the server makes: absent, it is kept.
      const draft = route.request().postDataJSON();
      mock.stationImages.push(draft.image_url);
      const previous = mock.stations.find((station) => station.id === id);
      const image =
        "image_url" in draft
          ? (draft.image_url ?? null)
          : (previous?.image_url ?? null);
      const replaced = {
        id,
        name: String(draft.name).trim(),
        stream_url: draft.stream_url,
        homepage_url: draft.homepage_url ?? null,
        image_url: image,
        artwork_hash: image ? imageHash(image) : null,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_001,
      };
      mock.stations = mock.stations.map((station) =>
        station.id === id ? replaced : station,
      );
      await route.fulfill({ json: replaced });
      return;
    }
    if (url.pathname === "/api/v2/bookmarks") {
      await route.fulfill({ json: mock.bookmarks });
      return;
    }
    if (url.pathname === "/api/v2/subsonic-credential") {
      const method = route.request().method();
      if (method !== "GET") {
        mock.subsonicWrites.push(method);
        await mock.subsonicGate;
      }
      if (method === "PUT") {
        // Rotation, never creation: an account holding none gets a 404, the
        // way the server answers it, so a panel that offered the control
        // anyway would fail here.
        if (mock.ownSubsonicCredential === null) {
          await route.fulfill({ status: 404, json: { error: "not found" } });
          return;
        }
        mock.ownSubsonicCredential = {
          ...mock.ownSubsonicCredential,
          updated_at: 1_700_990_000_000,
        };
        await route.fulfill({ json: { api_key: "wfsk_shown_once_only" } });
        return;
      }
      if (method === "DELETE") {
        mock.ownSubsonicCredential = null;
        await route.fulfill({ status: 204, body: "" });
        return;
      }
      await route.fulfill({ json: mock.ownSubsonicCredential });
      return;
    }
    // Before the administrative `/tokens` branch below, which would swallow
    // it: this path is the account asking about itself.
    if (url.pathname === "/api/v2/api-tokens") {
      if (route.request().method() === "POST") {
        mock.clientWrites.push("POST");
        await mock.clientGate;
        const asked = route.request().postDataJSON() as { name: string };
        const created: Client = {
          id: `client-${mock.ownClients.length + 1}`,
          name: asked.name,
          scopes: [],
          expires_at: 1_900_000_000_000,
          created_at: 1_700_900_000_000,
          last_used_at: null,
          revoked_at: null,
        };
        mock.ownClients = [created, ...mock.ownClients];
        await route.fulfill({
          status: 201,
          json: { ...created, secret: "wfapi_shown_once_and_never_again" },
        });
        return;
      }
      await route.fulfill({ json: mock.ownClients });
      return;
    }
    if (url.pathname.startsWith("/api/v2/api-tokens/")) {
      const id = url.pathname.split("/").pop();
      mock.ownClients = mock.ownClients.map((client) =>
        client.id === id
          ? { ...client, revoked_at: 1_700_950_000_000 }
          : client,
      );
      await route.fulfill({ status: 204, body: "" });
      return;
    }
    if (url.pathname.endsWith("/tokens")) {
      if (mock.tokensFail) {
        await route.fulfill({ status: 500, json: { error: "boom" } });
        return;
      }
      await route.fulfill({ json: [] });
      return;
    }
    if (url.pathname === "/api/v2/now-playing") {
      await route.fulfill({ json: [] });
      return;
    }
    if (url.pathname === "/api/v2/libraries") {
      await route.fulfill({ json: mock.libraries });
      return;
    }
    if (url.pathname === "/api/v2/queue") {
      await route.fulfill({
        json: {
          current: "song-1",
          position_ms: 0,
          changed_by: null,
          updated_at: 1,
          songs: albumDetail.songs,
        },
      });
      return;
    }
    // Scrobbling — RFC-010. The order of these four matters: the authorise
    // path and the retry path each sit where a destination name and an entry
    // id would otherwise be read, which is why the server puts the literal
    // before the parameter, and why the mock has to match in the same order.
    if (url.pathname === "/api/v2/scrobble-destinations") {
      await route.fulfill({ json: mock.destinations });
      return;
    }
    if (url.pathname === "/api/v2/scrobble-links") {
      await route.fulfill({ json: mock.scrobbleLinks });
      return;
    }
    if (url.pathname.startsWith("/api/v2/scrobble-links/lastfm/authorize/")) {
      // The verb as it arrived, not the verb expected. Written in, the
      // recording agreed with the assertion whatever the client had sent, and
      // the three branches that record a write could not fail for the reason
      // they exist.
      mock.scrobbleWrites.push({
        method: route.request().method(),
        path: url.pathname,
      });
      await route.fulfill({
        json: { authorize_url: mock.lastFmAuthorizeUrl, expires_in: 720 },
      });
      return;
    }
    if (url.pathname.startsWith("/api/v2/scrobble-links/")) {
      const [, , , , provider, destination] = url.pathname.split("/");
      const method = route.request().method();
      mock.scrobbleWrites.push({
        method,
        path: url.pathname,
        body: method === "PUT" ? route.request().postDataJSON() : undefined,
      });
      const recipient = provider as Recipient;
      // Both gestures address the pair. `PUT` replaces rather than adds, as
      // the route does: presenting a second token leaves one link, not two.
      const others = mock.scrobbleLinks.filter(
        (link) =>
          link.provider !== recipient || link.destination !== destination,
      );
      mock.scrobbleLinks =
        method === "PUT"
          ? [...others, linkState(recipient, destination)]
          : others;
      await route.fulfill({ status: 204, body: "" });
      return;
    }
    if (url.pathname.endsWith("/retry")) {
      const id = url.pathname.split("/")[5];
      mock.scrobbleWrites.push({
        method: route.request().method(),
        path: url.pathname,
      });
      mock.uncertain = mock.uncertain.filter((entry) => entry.id !== id);
      await route.fulfill({ json: { id: "entry-2" } });
      return;
    }
    if (url.pathname.startsWith("/api/v2/scrobble-queue/uncertain/")) {
      const id = url.pathname.split("/")[5];
      mock.scrobbleWrites.push({
        method: route.request().method(),
        path: url.pathname,
      });
      mock.uncertain = mock.uncertain.filter((entry) => entry.id !== id);
      await route.fulfill({ status: 204, body: "" });
      return;
    }
    if (url.pathname === "/api/v2/scrobble-queue/uncertain") {
      await route.fulfill({ json: mock.uncertain });
      return;
    }
    if (url.pathname === "/api/v2/history") {
      await route.fulfill({ json: history });
      return;
    }
    if (url.pathname === "/api/v2/libraries/library-1/uploads") {
      const { offers } = route.request().postDataJSON() as { offers: Offer[] };
      mock.uploadOffers.push(...offers);
      await route.fulfill({
        json: {
          verdicts: offers.map((offer) => ({
            full_hash: offer.full_hash,
            decision: "accepted",
            session: mock.uploadSession,
          })),
        },
      });
      return;
    }
    if (url.pathname === "/api/v2/uploads/upload-1") {
      await route.fulfill({ json: mock.uploadSession });
      return;
    }
    if (url.pathname.startsWith("/api/v2/uploads/upload-1/chunks/")) {
      const index = Number(url.pathname.split("/").pop());
      if (index !== mock.uploadSession.next_chunk) {
        await route.fulfill({ status: 409, json: { code: "conflict" } });
        return;
      }
      const body = route.request().postDataBuffer() ?? Buffer.alloc(0);
      mock.uploadChunks.push({ index, bytes: [...body] });
      mock.uploadSession = {
        ...mock.uploadSession,
        next_chunk: index + 1,
        received_bytes: mock.uploadSession.received_bytes + body.length,
      };
      if (index === mock.loseAcknowledgementOf) {
        mock.loseAcknowledgementOf = null;
        await route.fulfill({ status: 409, json: { code: "conflict" } });
        return;
      }
      await route.fulfill({ json: mock.uploadSession });
      return;
    }
    if (url.pathname === "/api/v2/uploads/upload-1/commit") {
      mock.uploadCommits.push("upload-1");
      await route.fulfill({
        status: 201,
        json: {
          track_id: "song-uploaded",
          full_hash: mock.uploadOffers[0]?.full_hash,
        },
      });
      return;
    }
    if (
      url.pathname.startsWith("/api/v2/tracks/") &&
      url.pathname.endsWith("/lyrics")
    ) {
      // The playing page asks for these, and the catch-all below would answer
      // with a track, which is not a list of lyrics — no test visited that page
      // before the canvas put a loop on it. Empty unless a test says otherwise.
      const trackId = url.pathname.split("/")[4] as string;
      await mock.lyricsGate;
      await route.fulfill({
        json: {
          trackId,
          structuredLyrics: mock.lyricSheets.get(trackId) ?? [],
        },
      });
      return;
    }
    const parts = url.pathname.split("/");
    if (
      parts.length === 6 &&
      parts[3] === "tracks" &&
      (parts[5] === "canvas" || parts[5] === "canvas-ticket")
    ) {
      const trackId = parts[4] as string;
      const method = route.request().method();
      if (parts[5] === "canvas-ticket") {
        // What the server does: minting checks the link, so a track without
        // a canvas answers 404 to the ticket itself.
        if (mock.canvases.has(trackId)) {
          await route.fulfill({
            json: {
              url: `/api/v2/canvas-stream/ticket-${trackId}`,
              expires_at: Date.now() + 3_600_000,
            },
          });
        } else {
          await route.fulfill({ status: 404, json: { error: "not found" } });
        }
        return;
      }
      if (method === "PUT") {
        const body = route.request().postDataBuffer() ?? Buffer.alloc(0);
        const bytes = [...body];
        mock.canvasWrites.push({ method, track: trackId, bytes });
        if (mock.refuseCanvasWith !== null) {
          const status = mock.refuseCanvasWith;
          mock.refuseCanvasWith = null;
          await route.fulfill({ status, json: { error: "refused" } });
          return;
        }
        mock.canvases.set(trackId, bytes);
        await route.fulfill({
          json: {
            url: "/api/v2/canvas/loop",
            hash: "loop",
            format: "webm",
            byte_size: bytes.length,
          },
        });
        return;
      }
      if (method === "DELETE") {
        mock.canvasWrites.push({ method, track: trackId });
        if (!mock.canvases.delete(trackId)) {
          await route.fulfill({ status: 404, json: { error: "not found" } });
          return;
        }
        await route.fulfill({ status: 204, body: "" });
        return;
      }
    }
    if (url.pathname.startsWith("/api/v2/canvas-stream/")) {
      await mock.canvasStreamGate;
      // The element that asked may be gone by the time the gate opens, and a
      // request it abandoned can no longer be answered.
      await route
        .fulfill({
          status: 200,
          headers: { "content-type": "video/webm" },
          body: "not a video",
        })
        .catch(() => undefined);
      return;
    }
    if (url.pathname === "/api/v2/tracks/song-9/overrides") {
      await route.fulfill({ json: mock.correctable.tracked });
      return;
    }
    if (url.pathname === "/api/v2/tracks/song-9") {
      if (route.request().method() === "PATCH") {
        const body = route.request().postDataJSON() as Record<string, unknown>;
        mock.corrections.push(body);
        // What the server does with it: a key present replaces its correction,
        // null removes it, and a list that is removed goes back to the file.
        Object.assign(mock.correctable.tracked.overrides, body);
        if (typeof body.title === "string")
          mock.correctable.song.title = body.title;
        if (body.artists === null) mock.correctable.song.artists = fileArtists;
      }
      await route.fulfill({ json: mock.correctable.song });
      return;
    }
    if (url.pathname.startsWith("/api/v2/tracks/")) {
      const id = url.pathname.split("/")[4];
      await route.fulfill({
        json: { ...song(1, `Track ${id}`, 0, false), id },
      });
      return;
    }
    // RFC-024: the statistics and the mixes, as the server shapes them.
    if (url.pathname === "/api/v2/stats") {
      const period = url.searchParams.get("period") ?? "30d";
      mock.statsAsks.push(period);
      await route.fulfill({ json: statsFor(period) });
      return;
    }
    if (url.pathname === "/api/v2/mixes") {
      await route.fulfill({ json: mock.mixes });
      return;
    }
    if (url.pathname.startsWith("/api/v2/mixes/")) {
      const id = decodeURIComponent(
        url.pathname.slice("/api/v2/mixes/".length),
      );
      mock.mixAsks.push(id);
      if (mock.mixFails) {
        await route.fulfill({ status: 500, json: { error: "internal" } });
        return;
      }
      const found = mock.mixes.find((mix) => mix.id === id);
      if (!found) {
        await route.fulfill({ status: 404, json: { error: "not found" } });
        return;
      }
      await route.fulfill({
        json: {
          ...found,
          songs: [
            song(1, "Hidden Place", 0, false),
            song(2, "Cocoon", 0, false),
          ],
        },
      });
      return;
    }
    if (url.pathname === "/api/v2/genres") {
      await route.fulfill({ json: genres });
      return;
    }
    if (url.pathname === "/api/v2/songs/by-genre") {
      await route.fulfill({
        json: url.searchParams.get("genre") ? genreSongs : [],
      });
      return;
    }
    if (url.pathname === "/api/v2/albums/album-2") {
      await route.fulfill({ json: albumDetail });
      return;
    }
    if (url.pathname.startsWith("/api/v2/ratings/")) {
      await route.fulfill({ status: 204, body: "" });
      return;
    }
    if (url.pathname === "/api/v2/playlists") {
      await route.fulfill({ json: playlists });
      return;
    }
    // RFC-025, answered as a server with none of either: the screens these
    // tests drive read them, and a 404 would draw a failure nobody asked for.
    // `smart-playlists-and-podcasts.spec.ts` exercises both.
    if (url.pathname === "/api/v2/smart-playlists") {
      await route.fulfill({ json: [] });
      return;
    }
    if (url.pathname === "/api/v2/podcasts") {
      await route.fulfill({
        json: { enabled: false, can_manage: false, channels: [] },
      });
      return;
    }
    if (url.pathname === "/api/v2/playlists/import") {
      mock.importAsks.push({
        dryRun: url.searchParams.get("dry_run") === "true",
        name: url.searchParams.get("name"),
      });
      const dryRun = url.searchParams.get("dry_run") === "true";
      await route.fulfill({
        status: dryRun ? 200 : 201,
        json: {
          ...importReport,
          name: url.searchParams.get("name") ?? importReport.name,
          playlist: dryRun
            ? null
            : { ...playlists[0], name: url.searchParams.get("name") },
        },
      });
      return;
    }
    if (url.pathname.endsWith("/export")) {
      mock.exportAsks.push(url.pathname);
      await route.fulfill({
        status: 200,
        headers: {
          "content-type": "audio/x-mpegurl; charset=utf-8",
          "content-disposition": 'attachment; filename="Evening.m3u8"',
        },
        body: "#EXTM3U\n#PLAYLIST:Evening\n#EXTINF:300,Björk - Hidden Place\nBjörk/Vespertine/01.flac\n",
      });
      return;
    }
    if (url.pathname === "/api/v2/artists") {
      await route.fulfill({
        json: [
          {
            id: artistDetail.id,
            name: artistDetail.name,
            album_count: artistDetail.album_count,
            artwork_hash: null,
          },
        ],
      });
      return;
    }
    if (url.pathname === "/api/v2/artists/artist-1/top-songs") {
      mock.topSongAsks.push(url.searchParams.get("limit"));
      await route.fulfill({ json: mock.artistPlayed });
      return;
    }
    if (url.pathname === "/api/v2/artists/artist-1/albums") {
      await route.fulfill({ json: albums });
      return;
    }
    if (url.pathname === "/api/v2/artists/artist-1") {
      mock.artistAsks += 1;
      if (mock.artistFailsAfterFirst && mock.artistAsks > 1) {
        await route.fulfill({ status: 500, json: { error: "internal" } });
        return;
      }
      await route.fulfill({
        json: {
          ...artistDetail,
          ...mock.artistExtra,
          enrichment: mock.enrichmentOf(mock.artistAsks),
        },
      });
      return;
    }
    if (url.pathname === "/api/v2/albums") {
      // Answering the order lets a test tell a refetch from a local re-sort.
      const sorted =
        url.searchParams.get("sort") === "newest"
          ? [...albums].reverse()
          : albums;
      await route.fulfill({ json: sorted });
      return;
    }
    if (
      url.pathname === "/api/v2/shares" &&
      route.request().method() === "GET"
    ) {
      // None yet. Answered so that the Shares section of Settings draws as a
      // page rather than as its failure when the hub's tests walk through it.
      await route.fulfill({ json: [] });
      return;
    }
    await route.fulfill({ status: 404, json: { error: "not found" } });
  });
}

/**
 * Before every test: the mock's state back to its defaults, and `/api/v2`
 * mocked over it. Each spec installs it with `test.beforeEach`.
 */
export const resetAndMockApi = async ({ page }: { page: Page }) => {
  mock.libraries = [library("library-1", "Ma musique")];
  mock.missingTracks = [];
  mock.purgedTracks = [];
  mock.purgeAsks = [];
  mock.restoreAsks = [];
  mock.membersFail = false;
  mock.tokensFail = false;
  mock.ownClients = freshClients();
  mock.bookmarks = [];
  mock.importAsks = [];
  mock.exportAsks = [];
  mock.stations = [];
  mock.radioWrites = [];
  mock.stationImages = [];
  mock.radioGate = Promise.resolve();
  mock.artistPlayed = [
    song(2, "Cocoon", 0, false),
    song(1, "Hidden Place", 5, true),
  ];
  mock.topSongAsks = [];
  mock.artistAsks = 0;
  mock.artistExtra = {};
  mock.enrichmentOf = () => undefined;
  mock.artistFailsAfterFirst = false;
  // **Reset here, and they were not.** These four reached the mock without
  // reaching this block, because a `replace` whose needle had already been
  // reformatted did nothing and said nothing. So `subsonicWrites`
  // accumulated across every test a worker ran, and the rotation assertion
  // passed only because its test and the one that writes before it happened to
  // land in different workers. A gate is worse: a test that fails before
  // releasing one leaves it pending, and every later test in that worker waits
  // on the route for ever.
  mock.subsonicWrites = [];
  mock.subsonicGate = Promise.resolve();
  mock.clientWrites = [];
  mock.clientGate = Promise.resolve();
  // An account the operator has granted one, because that is the state the
  // panel is about: it rotates and switches off, and never creates.
  mock.ownSubsonicCredential = {
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
  };
  mock.scanFails = false;
  mock.scanDrops = false;
  mock.corrections = [];
  mock.correctable = correctableTrack();
  mock.uploadOffers = [];
  mock.uploadChunks = [];
  mock.uploadCommits = [];
  mock.uploadSession = freshUploadSession();
  mock.loseAcknowledgementOf = null;
  mock.lyricSheets = new Map();
  mock.lyricsGate = Promise.resolve();
  mock.mixes = [];
  mock.mixAsks = [];
  mock.mixFails = false;
  mock.statsAsks = [];
  mock.canvases = new Map();
  mock.canvasWrites = [];
  mock.refuseCanvasWith = null;
  mock.canvasStreamGate = Promise.resolve();
  mock.slowAlbum = Promise.resolve();
  mock.destinations = freshDestinations();
  mock.scrobbleLinks = [];
  mock.uncertain = freshUncertain();
  mock.scrobbleWrites = [];
  mock.lastFmAuthorizeUrl = "https://www.last.fm/api/auth/?api_key=k&cb=back";
  await mockAuthenticatedApi(page);
};
