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
/** Where the module sent the browser, for the tests that care. */
let navigate: FetchStub;
const realLocation = window.location;

beforeEach(() => {
  fetchStub = vi.fn();
  vi.stubGlobal("fetch", fetchStub);
  // A session that ends sends the browser to the sign-in screen, and jsdom
  // implements no navigation: left alone it prints "Not implemented" over the
  // run and the destination cannot be asserted. Stood in for every test in
  // this file rather than the one that reads it, so no other has to know that
  // ending a session navigates.
  navigate = vi.fn();
  Object.defineProperty(window, "location", {
    configurable: true,
    value: { pathname: "/albums", assign: navigate },
  });
  // The refresh path reads the CSRF cookie, so a test has to plant one. The
  // Cookie Store API the rule suggests is not implemented in jsdom.
  // biome-ignore lint/suspicious/noDocumentCookie: seeding a cookie is the point
  document.cookie = "waveflow-csrf=csrf-token";
});

afterEach(() => {
  Object.defineProperty(window, "location", {
    configurable: true,
    value: realLocation,
  });
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

  it("ends the session when a renewal cannot make a 401 go away", async () => {
    const api = await freshApi();
    await signIn(api);
    fetchStub.mockImplementation(async (input: RequestInfo | URL) =>
      String(input).includes("/auth/refresh")
        ? jsonResponse({
            access_token: "second-token",
            user: { id: "u1", username: "dev", role: "admin" },
            device_id: "d1",
          })
        : jsonResponse({}, 401),
    );
    expect(api.hasSession()).toBe(true);

    await expect(api.getTrack("t1")).rejects.toThrow();

    // Rejecting and stopping there is what let a poll outlive its own session:
    // a query on `refetchInterval` keeps its schedule through an error, so
    // `now-playing` asked again every thirty seconds while the screen sat on a
    // library it could no longer read. The server spends 401 on a credential
    // it will not accept and nothing else — an authorisation refusal is 403,
    // or blurred into 404 — so there is nothing left to wait for.
    expect(api.hasSession()).toBe(false);
    expect(navigate).toHaveBeenCalledWith("/login");
  });

  it("leaves alone a session that began while an older refusal was out", async () => {
    const api = await freshApi();
    await signIn(api);
    let releaseRetry = () => {};
    const heldRetry = new Promise<Response>((resolve) => {
      releaseRetry = () => resolve(jsonResponse({}, 401));
    });
    let attempts = 0;
    fetchStub.mockImplementation(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/auth/refresh") || url.includes("/auth/login")) {
        return jsonResponse({
          access_token: "second-token",
          user: { id: "u1", username: "dev", role: "admin" },
          device_id: "d1",
        });
      }
      if (url.includes("/auth/logout"))
        return new Response(null, { status: 204 });
      attempts += 1;
      // The first answer starts a renewal; the second is held, so the sign-out
      // and sign-in below happen while it is still out.
      return attempts === 1 ? jsonResponse({}, 401) : heldRetry;
    });

    const refusedForTheFirst = api.getTrack("t1");
    await vi.waitFor(() => expect(attempts).toBe(2));
    await api.logout();
    await api.login("dev", "correct horse battery staple");
    expect(api.hasSession()).toBe(true);

    releaseRetry();
    await expect(refusedForTheFirst).rejects.toThrow();

    // A sign-out and a sign-in fit inside one round trip. Ending a session on
    // a refusal addressed to the account before it would have put whoever just
    // arrived back on the sign-in screen.
    expect(api.hasSession()).toBe(true);
    expect(navigate).not.toHaveBeenCalled();
  });

  it("leaves it alone for a caller that never asked for a renewal", async () => {
    const api = await freshApi();
    await signIn(api);
    fetchStub.mockResolvedValue(jsonResponse({}, 401));

    // `setupRequired` runs before anyone is signed in and switches the retry
    // off for that reason, so its 401 says nothing about a session. Reading
    // "the retry is off" as "a renewal was already spent" would have ended one
    // here — harmless while no session exists, and wrong all the same.
    await expect(api.setupRequired()).rejects.toThrow();

    expect(api.hasSession()).toBe(true);
    expect(navigate).not.toHaveBeenCalled();
  });

  it("ends it too on the one route that cannot go through `call`", async () => {
    const api = await freshApi();
    await signIn(api);
    fetchStub.mockImplementation(async (input: RequestInfo | URL) =>
      String(input).includes("/auth/refresh")
        ? jsonResponse({
            access_token: "second-token",
            user: { id: "u1", username: "dev", role: "admin" },
            device_id: "d1",
          })
        : jsonResponse({}, 401),
    );

    // A canvas goes whole, with its own content type, so `placeCanvas` does
    // its own renewal rather than borrowing the one above — and so it has to
    // leave the same dead end the same way. Covered here because inverting
    // that branch alone left every other test green.
    await expect(
      api.placeCanvas("t1", new Blob(["loop"], { type: "video/mp4" })),
    ).rejects.toThrow();

    expect(api.hasSession()).toBe(false);
    expect(navigate).toHaveBeenCalledWith("/login");
  });
});

/**
 * What the client remembers about a track that carries no loop.
 *
 * Minting a ticket is how the question is asked, so a track without a canvas
 * answers 404 to it. Both callers were right to distrust a *ticket* — it
 * expires — and neither kept the absence, so an ordinary library produced one
 * 404 per navigation without end.
 */
describe("a canvas the server says is not there", () => {
  const ticketRequests = () =>
    fetchStub.mock.calls.filter((call) =>
      String(call[0]).includes("/canvas-ticket"),
    );

  it("is asked about once, however many times a screen mounts", async () => {
    const api = await freshApi();
    await signIn(api);
    fetchStub.mockResolvedValue(jsonResponse({}, 404));

    await expect(api.canvasUrl("t1")).resolves.toBeNull();
    await expect(api.canvasUrl("t1")).resolves.toBeNull();
    await expect(api.canvasUrl("t1")).resolves.toBeNull();

    expect(ticketRequests()).toHaveLength(1);
  });

  it("is still asked about for a different track", async () => {
    const api = await freshApi();
    await signIn(api);
    fetchStub.mockResolvedValue(jsonResponse({}, 404));

    await expect(api.canvasUrl("t1")).resolves.toBeNull();
    await expect(api.canvasUrl("t2")).resolves.toBeNull();

    expect(ticketRequests()).toHaveLength(2);
  });

  it("is asked about again once this client places one", async () => {
    const api = await freshApi();
    await signIn(api);
    fetchStub.mockResolvedValue(jsonResponse({}, 404));
    await expect(api.canvasUrl("t1")).resolves.toBeNull();

    fetchStub.mockResolvedValueOnce(
      jsonResponse({ url: "/blob", hash: "h", format: "mp4", byte_size: 1 }),
    );
    await api.placeCanvas("t1", new Blob(["loop"], { type: "video/mp4" }));

    // Without forgetting, the panel that just placed a loop would read back
    // the answer from before it did.
    fetchStub.mockResolvedValueOnce(
      jsonResponse({ url: "/api/v2/canvas-stream/sealed", expires_at: 42 }),
    );
    await expect(api.canvasUrl("t1")).resolves.toEqual({
      url: "/api/v2/canvas-stream/sealed",
      expiresAt: 42,
    });
  });

  it("is asked about once by callers that ask at the same moment", async () => {
    const api = await freshApi();
    await signIn(api);
    fetchStub.mockResolvedValue(jsonResponse({}, 404));

    // `StrictMode` invokes an effect twice on mount, so the playing screen asks
    // twice for every loop in development — two questions in flight, neither
    // able to see the other's answer.
    const [first, second] = await Promise.all([
      api.canvasUrl("t1"),
      api.canvasUrl("t1"),
    ]);

    expect(first).toBeNull();
    expect(second).toBeNull();
    expect(ticketRequests()).toHaveLength(1);
  });

  it("is not recorded from an answer older than the session", async () => {
    const api = await freshApi();
    await signIn(api);
    let releaseTicket = () => {};
    const heldTicket = new Promise<Response>((resolve) => {
      releaseTicket = () => resolve(jsonResponse({}, 404));
    });
    fetchStub.mockImplementation(async (input: RequestInfo | URL) =>
      String(input).includes("/canvas-ticket")
        ? heldTicket
        : new Response(null, { status: 204 }),
    );

    const asking = api.canvasUrl("t1");
    await api.logout();
    releaseTicket();
    await expect(asking).resolves.toBeNull();

    // A 404 also means "none you may see". Keeping one account's refusal for
    // the next would hide a canvas the next account can read.
    fetchStub.mockResolvedValueOnce(
      jsonResponse({ url: "/api/v2/canvas-stream/sealed", expires_at: 42 }),
    );
    await expect(api.canvasUrl("t1")).resolves.toEqual({
      url: "/api/v2/canvas-stream/sealed",
      expiresAt: 42,
    });
  });

  it("is not recorded from an answer older than a placement", async () => {
    const api = await freshApi();
    await signIn(api);
    let releaseTicket = () => {};
    const heldTicket = new Promise<Response>((resolve) => {
      releaseTicket = () => resolve(jsonResponse({}, 404));
    });
    fetchStub.mockImplementation(async (input: RequestInfo | URL) =>
      String(input).includes("/canvas-ticket")
        ? heldTicket
        : jsonResponse({
            url: "/blob",
            hash: "h",
            format: "mp4",
            byte_size: 1,
          }),
    );

    // Asked before the placement, answered after it: the 404 is the truth of a
    // moment that has passed, and filing it would lose the loop just put there.
    const asking = api.canvasUrl("t1");
    await api.placeCanvas("t1", new Blob(["loop"], { type: "video/mp4" }));
    releaseTicket();
    await expect(asking).resolves.toBeNull();

    fetchStub.mockResolvedValueOnce(
      jsonResponse({ url: "/api/v2/canvas-stream/sealed", expires_at: 42 }),
    );
    await expect(api.canvasUrl("t1")).resolves.toEqual({
      url: "/api/v2/canvas-stream/sealed",
      expiresAt: 42,
    });
  });

  it("is dropped even by a placement the server refuses", async () => {
    const api = await freshApi();
    await signIn(api);
    fetchStub.mockResolvedValue(jsonResponse({}, 404));
    await expect(api.canvasUrl("t1")).resolves.toBeNull();

    // A refusal is not proof that nothing changed: a 404 on this route can
    // also be another client having moved the loop while this one looked at
    // it. Nothing is forgotten after a failure, so it has to happen before.
    fetchStub.mockResolvedValueOnce(jsonResponse({}, 409));
    await expect(
      api.placeCanvas("t1", new Blob(["loop"], { type: "video/mp4" })),
    ).rejects.toMatchObject({ status: 409 });

    fetchStub.mockResolvedValueOnce(
      jsonResponse({ url: "/api/v2/canvas-stream/sealed", expires_at: 42 }),
    );
    await expect(api.canvasUrl("t1")).resolves.toEqual({
      url: "/api/v2/canvas-stream/sealed",
      expiresAt: 42,
    });
  });

  it("is not recorded from an answer that raced a placement", async () => {
    const api = await freshApi();
    await signIn(api);
    let commitPlacement = () => {};
    const heldPlacement = new Promise<Response>((resolve) => {
      commitPlacement = () =>
        resolve(
          jsonResponse({
            url: "/blob",
            hash: "h",
            format: "mp4",
            byte_size: 1,
          }),
        );
    });
    fetchStub.mockImplementation(async (input: RequestInfo | URL) =>
      String(input).includes("/canvas-ticket")
        ? jsonResponse({}, 404)
        : heldPlacement,
    );

    const placing = api.placeCanvas(
      "t1",
      new Blob(["loop"], { type: "video/mp4" }),
    );
    // Asked after the placement began and answered before it was committed, so
    // this 404 is true at the moment it is given and stale by the time the
    // loop exists. Forgetting only before the request would have filed it.
    await expect(api.canvasUrl("t1")).resolves.toBeNull();
    commitPlacement();
    await placing;

    fetchStub.mockResolvedValueOnce(
      jsonResponse({ url: "/api/v2/canvas-stream/sealed", expires_at: 42 }),
    );
    await expect(api.canvasUrl("t1")).resolves.toEqual({
      url: "/api/v2/canvas-stream/sealed",
      expiresAt: 42,
    });
  });

  it("is forgotten when the session ends", async () => {
    const api = await freshApi();
    await signIn(api);
    fetchStub.mockResolvedValue(jsonResponse({}, 404));
    await expect(api.canvasUrl("t1")).resolves.toBeNull();
    expect(ticketRequests()).toHaveLength(1);

    fetchStub.mockResolvedValueOnce(new Response(null, { status: 204 }));
    await api.logout();

    // Signing in as somebody else on a shared browser must not inherit what
    // the last account was told, here as everywhere else in this module.
    await expect(api.canvasUrl("t1")).resolves.toBeNull();
    expect(ticketRequests()).toHaveLength(2);
  });
});
