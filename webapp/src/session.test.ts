import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * The session and paging layer, exercised against a stubbed fetch.
 *
 * These are the parts of the client that fail quietly rather than loudly: a
 * second page silently dropped looks like a small library, and a duplicated
 * refresh looks like a random logout. Neither surfaces as an error, so neither
 * is caught by using the app.
 *
 * Modules are re-imported per test because the session and the in-flight
 * refresh are module-level state.
 */

type FetchStub = ReturnType<typeof vi.fn>;

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

async function freshApi() {
  vi.resetModules();
  return import("./api");
}

let fetchStub: FetchStub;

beforeEach(() => {
  fetchStub = vi.fn();
  vi.stubGlobal("fetch", fetchStub);
  // The refresh path reads the CSRF cookie, so a test has to plant one. The
  // Cookie Store API the rule suggests is not implemented in jsdom.
  // biome-ignore lint/suspicious/noDocumentCookie: seeding a cookie is the point
  document.cookie = "waveflow-csrf=csrf-token";
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

async function signIn(api: Awaited<ReturnType<typeof freshApi>>) {
  fetchStub.mockResolvedValueOnce(
    jsonResponse({
      access_token: "first-token",
      user: { id: "u1", username: "dev", role: "admin" },
      device_id: "d1",
    }),
  );
  await api.login("dev", "correct horse battery staple");
  fetchStub.mockClear();
}

describe("paging", () => {
  it("keeps requesting while a page comes back full", async () => {
    const api = await freshApi();
    await signIn(api);
    // The server caps a page at 500, so a full page means "there may be more".
    const full = Array.from({ length: 500 }, (_, index) => ({
      id: `a${index}`,
    }));
    fetchStub
      .mockResolvedValueOnce(jsonResponse(full))
      .mockResolvedValueOnce(jsonResponse([{ id: "last" }]));

    const albums = await api.listAlbums();

    expect(albums).toHaveLength(501);
    expect(fetchStub).toHaveBeenCalledTimes(2);
    const [firstUrl, secondUrl] = fetchStub.mock.calls.map((call) => call[0]);
    expect(firstUrl).toContain("offset=0");
    expect(secondUrl).toContain("offset=500");
  });

  it("stops on a short page instead of requesting an empty one", async () => {
    const api = await freshApi();
    await signIn(api);
    fetchStub.mockResolvedValueOnce(jsonResponse([{ id: "only" }]));

    await expect(api.listAlbums()).resolves.toHaveLength(1);
    expect(fetchStub).toHaveBeenCalledTimes(1);
  });
});

describe("session refresh", () => {
  it("refreshes once for concurrent 401s", async () => {
    const api = await freshApi();
    await signIn(api);

    // Refresh tokens rotate: a second refresh would present a token the server
    // already retired and drop the session outright.
    fetchStub.mockImplementation(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/auth/refresh")) {
        return jsonResponse({
          access_token: "second-token",
          user: { id: "u1", username: "dev", role: "admin" },
          device_id: "d1",
        });
      }
      const attempts = fetchStub.mock.calls.filter(
        (call) => String(call[0]) === url,
      ).length;
      return attempts === 1 ? jsonResponse({}, 401) : jsonResponse({ id: url });
    });

    await Promise.all([
      api.getTrack("one"),
      api.getTrack("two"),
      api.getTrack("three"),
    ]);

    const refreshes = fetchStub.mock.calls.filter((call) =>
      String(call[0]).includes("/auth/refresh"),
    );
    expect(refreshes).toHaveLength(1);
  });

  it("retries a 401 once and carries the new token", async () => {
    const api = await freshApi();
    await signIn(api);
    fetchStub
      .mockResolvedValueOnce(jsonResponse({}, 401))
      .mockResolvedValueOnce(
        jsonResponse({
          access_token: "second-token",
          user: { id: "u1", username: "dev", role: "admin" },
          device_id: "d1",
        }),
      )
      .mockResolvedValueOnce(jsonResponse({ id: "t1" }));

    await expect(api.getTrack("t1")).resolves.toEqual({ id: "t1" });

    const retry = fetchStub.mock.calls.at(-1);
    const headers = new Headers(retry?.[1]?.headers);
    expect(headers.get("authorization")).toBe("Bearer second-token");
  });

  it("gives up after one retry rather than looping", async () => {
    const api = await freshApi();
    await signIn(api);
    // A token that refreshes cleanly but is still refused must not drive an
    // endless refresh/retry cycle.
    fetchStub.mockImplementation(async (input: RequestInfo | URL) =>
      String(input).includes("/auth/refresh")
        ? jsonResponse({
            access_token: "second-token",
            user: { id: "u1", username: "dev", role: "admin" },
            device_id: "d1",
          })
        : jsonResponse({}, 401),
    );

    await expect(api.getTrack("t1")).rejects.toThrow();
    const requests = fetchStub.mock.calls.filter(
      (call) => !String(call[0]).includes("/auth/refresh"),
    );
    expect(requests).toHaveLength(2);
  });
});
