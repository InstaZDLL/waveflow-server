/*
 * The WaveFlow service worker (RFC-022, offline listening).
 *
 * It keeps two things and nothing else:
 *
 * - **the application shell** — the page and the content-hashed files it is
 *   built of — so the client opens without a network;
 * - **the tracks a listener chose to keep for offline**, stored by the page
 *   under `/offline/tracks/{track id}`, an address of this worker's own that
 *   names the track and carries no credential, in the cache of the account
 *   that kept them. The worker answers that address only from the cache of
 *   the account the owner marker names — the one whose session this browser
 *   holds or last held — so it never hands one account's copy to another;
 *   with no marker, after a sign-out, it answers none.
 *
 * **It never answers, and never stores, anything of the server's.** Every
 * request under `/api/`, `/rest/` and `/share/`, every probe and the API
 * reference, every request that is not a GET, and every request to another
 * origin goes to the network untouched: an API response, a token, a stream
 * ticket or a share's pass never reaches a cache through this file. A stream
 * ticket's address is never a key; a kept track is fetched through one by
 * the page and stored under its id.
 *
 * Plain JavaScript in `public/`, so it is served at `/sw.js` under a name
 * that does not change between builds, with the scope of the whole client.
 * `classify` is exposed on `self` for the client's tests.
 */

const SHELL_CACHE = "waveflow-shell-v1";
const ASSET_CACHE = "waveflow-assets-v1";
const OFFLINE_PREFIX = "waveflow-offline-v1:";
const OWNER_CACHE = "waveflow-offline-owner";
const OWNER_KEY = "/offline/owner";
const SHELL_KEY = "/";
const KNOWN_CACHES = [SHELL_CACHE, ASSET_CACHE, OWNER_CACHE];

/** Paths that belong to the server: never answered from a cache. */
const SERVER_PREFIXES = ["/api/", "/rest/", "/share/"];
const SERVER_EXACT = ["/health", "/ready", "/openapi.json", "/reference", "/sw.js"];

/**
 * What the worker does with one request: `network` (leave it alone),
 * `shell` (a page: network first, the kept shell offline), `asset` (a
 * content-hashed file: the cache first), `static` (an unhashed file of the
 * client: the network first, the cache offline) or `offline-track`.
 */
function classify(url, method, mode, origin) {
  if (method !== "GET") return "network";
  if (url.origin !== origin) return "network";
  const path = url.pathname;
  if (SERVER_PREFIXES.some((prefix) => path === prefix.slice(0, -1) || path.startsWith(prefix))) {
    return "network";
  }
  if (SERVER_EXACT.includes(path)) return "network";
  if (path.startsWith("/offline/tracks/")) return "offline-track";
  if (path.startsWith("/offline/meta/")) return "network";
  // A query string is the server's business — a ticket or a pass rides in one
  // — and the client's own files never carry one.
  if (url.search !== "") return "network";
  if (mode === "navigate") return "shell";
  if (path.startsWith("/assets/")) return "asset";
  if (/\.(?:svg|png|webmanifest|woff2?)$/.test(path)) return "static";
  return "network";
}

self.classify = classify;

self.addEventListener("install", (event) => {
  event.waitUntil(self.skipWaiting());
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      for (const name of await caches.keys()) {
        if (
          name.startsWith("waveflow-") &&
          !KNOWN_CACHES.includes(name) &&
          !name.startsWith(OFFLINE_PREFIX)
        ) {
          await caches.delete(name);
        }
      }
      await self.clients.claim();
    })(),
  );
});

self.addEventListener("fetch", (event) => {
  const request = event.request;
  const url = new URL(request.url);
  const kind = classify(url, request.method, request.mode, self.location.origin);
  if (kind === "network") return;
  if (kind === "shell") event.respondWith(page(request));
  else if (kind === "asset") event.respondWith(asset(request));
  else if (kind === "static") event.respondWith(staticFile(request));
  else if (kind === "offline-track") event.respondWith(offlineTrack(request));
});

/**
 * A page: the network, and a copy of the shell kept for when there is none.
 * Every client route answers the same shell, so one copy serves them all.
 * When the shell changes — a deploy — the hashed files the old one named and
 * the new one does not are let go, so the cache holds one build.
 */
async function page(request) {
  let response;
  try {
    response = await fetch(request);
  } catch {
    const kept = await caches.match(SHELL_KEY, { cacheName: SHELL_CACHE });
    return kept || unreachable();
  }
  const type = response.headers.get("content-type") || "";
  if (response.status === 200 && type.startsWith("text/html")) {
    // A copy that cannot be kept — the storage full — costs the next offline
    // visit, never this answer.
    await remember(async () => {
      const text = await response.clone().text();
      const shell = await caches.open(SHELL_CACHE);
      const previous = await shell.match(SHELL_KEY);
      await shell.put(SHELL_KEY, response.clone());
      if (!previous || (await previous.text()) !== text) await pruneAssets(text);
    });
  }
  return response;
}

/** Keeps a copy, and never lets a failure to keep it reach the answer. */
async function remember(keep) {
  try {
    await keep();
  } catch {
    // Quota, an eviction under way: the network answer stands.
  }
}

/** What a request gets with no network and nothing kept for it. */
function unreachable() {
  return new Response("Offline, and nothing kept for this address.", {
    status: 503,
    headers: { "content-type": "text/plain; charset=utf-8" },
  });
}

async function pruneAssets(shellText) {
  const assets = await caches.open(ASSET_CACHE);
  for (const request of await assets.keys()) {
    const path = new URL(request.url).pathname;
    if (!shellText.includes(path)) await assets.delete(request);
  }
}

/** A content-hashed file: its name is its content, so a kept copy is right. */
async function asset(request) {
  const kept = await caches.match(request, { cacheName: ASSET_CACHE });
  if (kept) return kept;
  let response;
  try {
    response = await fetch(request);
  } catch {
    return unreachable();
  }
  if (response.status === 200) {
    await remember(async () => {
      const assets = await caches.open(ASSET_CACHE);
      await assets.put(request, response.clone());
    });
  }
  return response;
}

/** An icon, the manifest, a font outside the build: fresh when possible. */
async function staticFile(request) {
  let response;
  try {
    response = await fetch(request);
  } catch {
    const kept = await caches.match(request, { cacheName: SHELL_CACHE });
    return kept || unreachable();
  }
  if (response.status === 200) {
    await remember(async () => {
      const shell = await caches.open(SHELL_CACHE);
      await shell.put(request, response.clone());
    });
  }
  return response;
}

/**
 * A kept track, with byte ranges: an `<audio>` element seeks by range, and a
 * cached response holds the whole file.
 */
async function offlineTrack(request) {
  const marker = await caches.match(OWNER_KEY, { cacheName: OWNER_CACHE });
  const owner = marker ? (await marker.text()).trim() : "";
  const kept = owner
    ? await caches.match(new URL(request.url).pathname, {
        cacheName: OFFLINE_PREFIX + owner,
      })
    : undefined;
  if (!kept) return new Response("Not kept for offline", { status: 404 });
  const range = request.headers.get("range");
  const type = kept.headers.get("content-type") || "application/octet-stream";
  const body = await kept.blob();
  const size = body.size;
  const match = range ? /^bytes=(\d*)-(\d*)$/.exec(range.trim()) : null;
  if (!match || (match[1] === "" && match[2] === "")) {
    return new Response(body, {
      status: 200,
      headers: {
        "content-type": type,
        "content-length": String(size),
        "accept-ranges": "bytes",
      },
    });
  }
  let start;
  let end;
  if (match[1] === "") {
    start = Math.max(0, size - Number(match[2]));
    end = size - 1;
  } else {
    start = Number(match[1]);
    end = match[2] === "" ? size - 1 : Math.min(Number(match[2]), size - 1);
  }
  if (start >= size || start > end) {
    return new Response(null, {
      status: 416,
      headers: { "content-range": `bytes */${size}` },
    });
  }
  return new Response(body.slice(start, end + 1), {
    status: 206,
    headers: {
      "content-type": type,
      "content-length": String(end - start + 1),
      "content-range": `bytes ${start}-${end}/${size}`,
      "accept-ranges": "bytes",
    },
  });
}
