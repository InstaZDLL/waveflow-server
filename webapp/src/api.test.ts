import { afterEach, describe, expect, it, vi } from "vitest";

import {
  hasSession,
  isAllowedRedirect,
  listAlbums,
  listLibraries,
  login,
  safeInternalPath,
} from "./api";

afterEach(() => {
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
