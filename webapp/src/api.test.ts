import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  canvasUrl,
  hasSession,
  isAllowedRedirect,
  listAlbums,
  listLibraries,
  login,
  logout,
  safeInternalPath,
  search,
} from "./api";

/**
 * Where the module sent the browser.
 *
 * A session that ends navigates to the sign-in screen, and jsdom implements no
 * navigation — left alone it prints "Not implemented" over the run. Only
 * `assign` is stood in for: `pathname` still reads the real location, because
 * the tests below drive it with `history.replaceState` and the module decides
 * whether to navigate by reading it.
 */
const realLocation = window.location;
let navigate: ReturnType<typeof vi.fn>;

beforeEach(() => {
  navigate = vi.fn();
  Object.defineProperty(window, "location", {
    configurable: true,
    value: {
      get pathname() {
        return realLocation.pathname;
      },
      assign: navigate,
    },
  });
});

afterEach(() => {
  Object.defineProperty(window, "location", {
    configurable: true,
    value: realLocation,
  });
  vi.unstubAllGlobals();
  // biome-ignore lint/suspicious/noDocumentCookie: jsdom has no Cookie Store API.
  document.cookie = "waveflow-csrf=; Max-Age=0; Path=/";
});

/**
 * These two guards are the client half of the OAuth redirect policy. Both were
 * added after review found the consent screen and the post-login hop navigating
 * to attacker-supplied targets, so they are covered here rather than left to
 * inspection.
 */
describe("isAllowedRedirect", () => {
  it("accepts what a native client can prove it controls", () => {
    expect(isAllowedRedirect("http://127.0.0.1:49152/cb")).toBe(true);
    expect(isAllowedRedirect("http://localhost:1234/cb")).toBe(true);
    expect(isAllowedRedirect("http://[::1]:5000/cb")).toBe(true);
    expect(isAllowedRedirect("https://desktop.example.com/cb")).toBe(true);
    expect(isAllowedRedirect("com.waveflow.desktop://auth")).toBe(true);
  });

  it("refuses targets that would carry the code elsewhere", () => {
    // Clear text to a remote host leaks the code off the machine.
    expect(isAllowedRedirect("http://evil.example.com/cb")).toBe(false);
    // A bare scheme is claimable by any other application.
    expect(isAllowedRedirect("waveflow://auth")).toBe(false);
    expect(isAllowedRedirect("javascript:alert(1)")).toBe(false);
    expect(isAllowedRedirect("not a url")).toBe(false);
    expect(isAllowedRedirect("")).toBe(false);
  });

  it("refuses a fragment, which the server also rejects", () => {
    expect(isAllowedRedirect("https://desktop.example.com/cb#frag")).toBe(
      false,
    );
  });
});

describe("safeInternalPath", () => {
  it("keeps a same-document path", () => {
    expect(safeInternalPath("/albums")).toBe("/albums");
    expect(safeInternalPath("/authorize?client_id=x&state=y")).toBe(
      "/authorize?client_id=x&state=y",
    );
  });

  it("refuses anything that leaves the origin", () => {
    // Protocol-relative: location.assign would follow it to another host.
    expect(safeInternalPath("//evil.example.com")).toBeNull();
    // Browsers normalise backslashes, so this escapes just like "//".
    expect(safeInternalPath("/\\evil.example.com")).toBeNull();
    expect(safeInternalPath("https://evil.example.com")).toBeNull();
    expect(safeInternalPath("javascript:alert(1)")).toBeNull();
    expect(safeInternalPath("albums")).toBeNull();
    expect(safeInternalPath(null)).toBeNull();
    expect(safeInternalPath("")).toBeNull();
  });
});

describe("session refresh failures", () => {
  const webSession = {
    access_token: "test-access",
    user: { id: "user-id", username: "listener", role: "user" },
    device_id: "device-id",
  };

  async function establishSession(refreshResult: () => Promise<Response>) {
    window.history.replaceState(null, "", "/login");
    // biome-ignore lint/suspicious/noDocumentCookie: jsdom has no Cookie Store API.
    document.cookie = "waveflow-csrf=test-csrf; Path=/";
    vi.stubGlobal(
      "fetch",
      vi
        .fn()
        .mockResolvedValueOnce(
          new Response(JSON.stringify(webSession), { status: 200 }),
        )
        .mockResolvedValueOnce(new Response(null, { status: 401 }))
        .mockImplementationOnce(refreshResult),
    );
    await login("listener", "password");
    expect(hasSession()).toBe(true);
  }

  it("clears an established session when refresh loses the network", async () => {
    await establishSession(() =>
      Promise.reject(new TypeError("network unavailable")),
    );

    await expect(listLibraries()).rejects.toMatchObject({ status: 401 });
    expect(hasSession()).toBe(false);
  });

  /**
   * A renewal takes a round trip, and a sign-out during that round trip used to
   * be undone by it: `logout` cleared the session, the answer arrived
   * afterwards, and the assignment put the previous account's token straight
   * back. The visitor was on the sign-in screen and still authenticated.
   *
   * Asserted on `hasSession` and not on the rejection: the call fails either
   * way — the retry meets the same 401 — and only the session says whether the
   * sign-out took.
   */
  it("lets a sign-out stand even when a renewal was already in flight", async () => {
    window.history.replaceState(null, "", "/");
    // biome-ignore lint/suspicious/noDocumentCookie: jsdom has no Cookie Store API.
    document.cookie = "waveflow-csrf=test-csrf; Path=/";
    let answerRefresh = () => {};
    const heldRefresh = new Promise<Response>((resolve) => {
      answerRefresh = () =>
        resolve(new Response(JSON.stringify(webSession), { status: 200 }));
    });
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/web/auth/login")) {
          return Promise.resolve(
            new Response(JSON.stringify(webSession), { status: 200 }),
          );
        }
        // Held, so the sign-out below happens while it is still out.
        if (url.endsWith("/web/auth/refresh")) return heldRefresh;
        if (url.endsWith("/web/auth/logout")) {
          return Promise.resolve(new Response(null, { status: 204 }));
        }
        return Promise.resolve(new Response(null, { status: 401 }));
      }),
    );

    await login("listener", "password");
    expect(hasSession()).toBe(true);

    const meetingA401 = listLibraries();
    await logout();
    expect(hasSession()).toBe(false);

    answerRefresh();
    await expect(meetingA401).rejects.toMatchObject({ status: 401 });
    // The renewal answered for an account that had already left.
    expect(hasSession()).toBe(false);
  });

  /**
   * The renewal in flight is shared between callers so a rotating token is not
   * spent twice — but only between callers of the same session. One that
   * outlived the account it was started for answers `false` on purpose, and
   * handing that answer to somebody who asked after a new session began would
   * fail their request for a reason that had stopped applying.
   */
  it("does not hand a renewal from a session that ended to the next one", async () => {
    window.history.replaceState(null, "", "/");
    // biome-ignore lint/suspicious/noDocumentCookie: jsdom has no Cookie Store API.
    document.cookie = "waveflow-csrf=test-csrf; Path=/";
    let renewals = 0;
    let answerFirst = () => {};
    const heldRefresh = new Promise<Response>((resolve) => {
      answerFirst = () =>
        resolve(new Response(JSON.stringify(webSession), { status: 200 }));
    });
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/web/auth/login")) {
          return Promise.resolve(
            new Response(JSON.stringify(webSession), { status: 200 }),
          );
        }
        if (url.endsWith("/web/auth/refresh")) {
          renewals += 1;
          // The first is held; anything after it answers at once.
          return renewals === 1
            ? heldRefresh
            : Promise.resolve(
                new Response(JSON.stringify(webSession), { status: 200 }),
              );
        }
        if (url.endsWith("/web/auth/logout")) {
          return Promise.resolve(new Response(null, { status: 204 }));
        }
        return Promise.resolve(new Response(null, { status: 401 }));
      }),
    );

    await login("listener", "password");
    const duringFirstSession = listLibraries();
    // Waited for rather than assumed: the 401 and the renewal it starts are a
    // few microtasks away from the call itself.
    await vi.waitFor(() => expect(renewals).toBe(1));

    await logout();
    // A caller after the session changed must not be given the renewal that is
    // still out for the one that left.
    const afterItEnded = listLibraries();
    await expect(afterItEnded).rejects.toMatchObject({ status: 401 });
    expect(renewals).toBe(2);

    answerFirst();
    await expect(duringFirstSession).rejects.toMatchObject({ status: 401 });
  });

  it("clears an established session when refresh JSON is malformed", async () => {
    await establishSession(() =>
      Promise.resolve(new Response("not-json", { status: 200 })),
    );

    await expect(listLibraries()).rejects.toMatchObject({ status: 401 });
    expect(hasSession()).toBe(false);
  });
});

/**
 * `collect` grew a parameter bag so the albums page could ask the server for an
 * order instead of sorting a page of the catalogue in the browser. Paging is
 * the part worth pinning: the order has to travel on *every* request, not only
 * the first, or the second page comes back sorted differently from the first.
 */
describe("listAlbums", () => {
  const album = (id: number) => ({ id: `album-${id}`, title: `Album ${id}` });

  /** Answers `pages` in turn and records every URL it was asked for. */
  function stubPages(pages: unknown[][]) {
    const urls: string[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn((url: string) => {
        urls.push(url);
        const page = pages[urls.length - 1] ?? [];
        return Promise.resolve(
          new Response(JSON.stringify(page), { status: 200 }),
        );
      }),
    );
    return urls;
  }

  it("carries the order onto every page, not just the first", async () => {
    const full = Array.from({ length: 500 }, (_, index) => album(index));
    const urls = stubPages([full, [album(500)]]);

    const albums = await listAlbums("newest");

    expect(urls).toHaveLength(2);
    for (const url of urls) {
      expect(new URLSearchParams(url.split("?")[1]).get("sort")).toBe("newest");
    }
    expect(albums).toHaveLength(501);
    // Order survives the concatenation: the server sorted, the client appended.
    expect(albums[0]?.id).toBe("album-0");
    expect(albums[500]?.id).toBe("album-500");
  });

  it("offsets each page by the page size", async () => {
    const full = Array.from({ length: 500 }, (_, index) => album(index));
    const urls = stubPages([full, []]);

    await listAlbums("newest");

    const offsets = urls.map((url) =>
      new URLSearchParams(url.split("?")[1]).get("offset"),
    );
    expect(offsets).toEqual(["0", "500"]);
  });

  it("sends no order when none is chosen, leaving the server default", async () => {
    const urls = stubPages([[album(1)]]);

    await listAlbums();

    expect(urls).toHaveLength(1);
    expect(new URLSearchParams(urls[0]?.split("?")[1]).has("sort")).toBe(false);
  });
});

/**
 * Search became scopeable on 2026-09-14, and the screen stopped warning that
 * it was not. Whether the scope actually travels is the whole of that change
 * from here: a client that dropped `library_id` would show the same reassuring
 * sentence over results from every library.
 */
describe("search", () => {
  function stubOnce() {
    const urls: string[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn((url: string) => {
        urls.push(url);
        return Promise.resolve(
          new Response(JSON.stringify({ artists: [], albums: [], songs: [] }), {
            status: 200,
          }),
        );
      }),
    );
    return urls;
  }

  it("carries the active library", async () => {
    const urls = stubOnce();

    await search("beacon", "lib-1");

    const query = new URLSearchParams(urls[0]?.split("?")[1]);
    expect(query.get("q")).toBe("beacon");
    expect(query.get("library_id")).toBe("lib-1");
  });

  it("sends none when the account is not scoped to one", async () => {
    // An account with no library, or a listing that could not be read: the
    // route treats an absent `library_id` as every library it may see, so
    // sending an empty one would be asking for a library named "".
    const urls = stubOnce();

    await search("beacon");

    const query = new URLSearchParams(urls[0]?.split("?")[1]);
    expect(query.get("q")).toBe("beacon");
    expect(query.has("library_id")).toBe(false);
  });

  it("escapes a query the URL would otherwise read as structure", async () => {
    const urls = stubOnce();

    await search("rock & roll?a=b", "lib-1");

    const query = new URLSearchParams(urls[0]?.split("?")[1]);
    expect(query.get("q")).toBe("rock & roll?a=b");
    expect(query.get("library_id")).toBe("lib-1");
  });
});

/**
 * The ticket is how the web client asks whether a track has a canvas, so its
 * two answers have to stay apart. A 404 is the server saying there is none;
 * anything else is the server not answering, and reading that as none would
 * quietly stop offering the removal of a loop that is still there.
 */
describe("canvasUrl", () => {
  /** Answers every request with `response`, recording the URLs asked for. */
  function answer(response: Response) {
    const urls: string[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn((url: string) => {
        urls.push(url);
        return Promise.resolve(response);
      }),
    );
    return urls;
  }

  // A track per case, deliberately. What the module remembers about a track
  // outlives a test here — this file imports it once rather than reloading it —
  // and a 404 is now remembered, so sharing an identifier would make each of
  // these depend on the ones before it. What that memory does is covered in
  // `session.test.ts`, where a fresh module per test can show it.

  it("mints a ticket for the track and hands back its URL", async () => {
    const urls = answer(
      new Response(
        JSON.stringify({ url: "/api/v2/canvas-stream/sealed", expires_at: 42 }),
        { status: 200 },
      ),
    );

    await expect(canvasUrl("has-one")).resolves.toEqual({
      url: "/api/v2/canvas-stream/sealed",
      expiresAt: 42,
    });
    expect(urls).toEqual(["/api/v2/tracks/has-one/canvas-ticket"]);
  });

  it("reads a 404 as a track without a canvas", async () => {
    answer(new Response(null, { status: 404 }));

    await expect(canvasUrl("has-none")).resolves.toBeNull();
  });

  it("throws a failure rather than calling it no canvas", async () => {
    answer(new Response(null, { status: 503 }));

    await expect(canvasUrl("unanswerable")).rejects.toMatchObject({
      status: 503,
    });
  });
});
