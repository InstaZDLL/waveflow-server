// @vitest-environment node
import { describe, expect, it } from "vitest";

import source from "../../public/sw.js?raw";
import { FakeCaches } from "./fake-caches";

type Listener = (event: unknown) => void;

/**
 * The worker as the browser would run it, against a cache and a network of
 * the test's own: `public/sw.js` is evaluated with `self`, `caches` and
 * `fetch` handed in, and its fetch listener is driven with requests.
 */
function boot(network: (request: Request) => Promise<Response>) {
  const listeners = new Map<string, Listener>();
  const store = new FakeCaches();
  const worker = {
    location: { origin: "http://client.test" },
    addEventListener: (type: string, listener: Listener) =>
      listeners.set(type, listener),
    skipWaiting: async () => undefined,
    clients: { claim: async () => undefined },
    classify: undefined as unknown,
  };
  new Function("self", "caches", "fetch", "Response", source)(
    worker,
    store,
    network,
    Response,
  );
  const classify = worker.classify as (
    url: URL,
    method: string,
    mode: string,
    origin: string,
  ) => string;
  /** What the worker answers, or `null` when it leaves the request alone. */
  async function request(
    path: string,
    init: RequestInit & { mode?: string } = {},
  ): Promise<Response | null> {
    const { mode, ...rest } = init;
    const request = new Request(`http://client.test${path}`, rest);
    // A navigation cannot be constructed, only received: its mode is set on
    // the instance, where the worker reads it.
    if (mode) Object.defineProperty(request, "mode", { value: mode });
    let answered: Promise<Response> | null = null;
    listeners.get("fetch")?.({
      request,
      respondWith: (response: Promise<Response>) => {
        answered = response;
      },
    });
    return answered;
  }
  return { classify, request, store };
}

const origin = "http://client.test";
const url = (path: string) => new URL(path, origin);

describe("the service worker", () => {
  it("leaves everything of the server's to the network", () => {
    const { classify } = boot(async () => new Response());
    for (const path of [
      "/api/v2/albums",
      "/api/v2/stream/wft_ticket",
      "/rest/stream.view?id=1",
      "/share/wfs_token",
      "/share/wfs_token/tracks/1/stream?visit=pass",
      "/health",
      "/ready",
      "/openapi.json",
      "/reference",
      "/sw.js",
      "/offline/meta/t1",
    ]) {
      expect(classify(url(path), "GET", "cors", origin), path).toBe("network");
      expect(classify(url(path), "GET", "navigate", origin), path).toBe(
        "network",
      );
    }
    expect(classify(url("/albums"), "POST", "navigate", origin)).toBe(
      "network",
    );
    expect(
      classify(new URL("https://elsewhere.test/a.js"), "GET", "cors", origin),
    ).toBe("network");
    expect(
      classify(url("/assets/index-1a2b.js?x=1"), "GET", "cors", origin),
    ).toBe("network");
  });

  it("keeps the shell, the hashed files and the kept tracks", () => {
    const { classify } = boot(async () => new Response());
    expect(classify(url("/albums/123"), "GET", "navigate", origin)).toBe(
      "shell",
    );
    expect(classify(url("/assets/index-1a2b.js"), "GET", "cors", origin)).toBe(
      "asset",
    );
    expect(classify(url("/icons/icon-192.png"), "GET", "cors", origin)).toBe(
      "static",
    );
    expect(classify(url("/offline/tracks/t1"), "GET", "cors", origin)).toBe(
      "offline-track",
    );
  });

  it("never stores an API answer, and opens the kept shell offline", async () => {
    let online = true;
    const { request, store } = boot(async (asked) => {
      if (!online) throw new TypeError("offline");
      if (new URL(asked.url).pathname.startsWith("/api/")) {
        return new Response('{"token":"secret"}', {
          headers: { "content-type": "application/json" },
        });
      }
      return new Response("<html>shell</html>", {
        headers: { "content-type": "text/html" },
      });
    });
    expect(await request("/api/v2/web/auth/refresh")).toBeNull();
    const page = await request("/albums", { mode: "navigate" });
    expect(await page?.text()).toBe("<html>shell</html>");
    expect(store.paths()).toEqual(["/"]);
    online = false;
    const offline = await request("/playlists", { mode: "navigate" });
    expect(await offline?.text()).toBe("<html>shell</html>");
    expect(store.paths().some((path) => path.startsWith("/api/"))).toBe(false);
  });

  it("answers a kept track by byte range", async () => {
    const { request, store } = boot(async () => new Response());
    const cache = await store.open("waveflow-offline-v1");
    await cache.put(
      "/offline/tracks/t1",
      new Response(new Blob([new Uint8Array([0, 1, 2, 3, 4, 5, 6, 7, 8, 9])]), {
        headers: { "content-type": "audio/flac" },
      }),
    );
    const whole = await request("/offline/tracks/t1");
    expect(whole?.status).toBe(200);
    expect(whole?.headers.get("accept-ranges")).toBe("bytes");
    const part = await request("/offline/tracks/t1", {
      headers: { range: "bytes=2-4" },
    });
    expect(part?.status).toBe(206);
    expect(part?.headers.get("content-range")).toBe("bytes 2-4/10");
    expect(new Uint8Array((await part?.arrayBuffer()) ?? [])).toEqual(
      new Uint8Array([2, 3, 4]),
    );
    const tail = await request("/offline/tracks/t1", {
      headers: { range: "bytes=-3" },
    });
    expect(new Uint8Array((await tail?.arrayBuffer()) ?? [])).toEqual(
      new Uint8Array([7, 8, 9]),
    );
    const beyond = await request("/offline/tracks/t1", {
      headers: { range: "bytes=20-" },
    });
    expect(beyond?.status).toBe(416);
    const missing = await request("/offline/tracks/nope");
    expect(missing?.status).toBe(404);
  });
});
