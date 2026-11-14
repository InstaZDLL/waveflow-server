import { afterEach, describe, expect, it, vi } from "vitest";

import { clipUrl } from "./index";

/** Answers every request with `response`, and records where they went. */
function answer(response: () => Response) {
  const urls: string[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn((url: string) => {
      urls.push(url);
      return Promise.resolve(response());
    }),
  );
  return urls;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

/**
 * A track's music video (RFC-029): the ticket is asked for each time, and only
 * the server's 404 means there is none.
 */
describe("clipUrl", () => {
  it("mints a clip ticket for the track and hands back its URL", async () => {
    const urls = answer(
      () =>
        new Response(
          JSON.stringify({ url: "/api/v2/clip-stream/sealed", expires_at: 42 }),
          { status: 200 },
        ),
    );

    await expect(clipUrl("has-one")).resolves.toEqual({
      url: "/api/v2/clip-stream/sealed",
      expiresAt: 42,
    });
    expect(urls).toEqual(["/api/v2/tracks/has-one/clip-ticket"]);
  });

  it("asks again every time, since a ticket is not a property of the track", async () => {
    const urls = answer(
      () =>
        new Response(
          JSON.stringify({ url: "/api/v2/clip-stream/sealed", expires_at: 42 }),
          { status: 200 },
        ),
    );

    await clipUrl("twice");
    await clipUrl("twice");
    expect(urls).toHaveLength(2);
  });

  it("reads a 404 as a track without a clip", async () => {
    answer(() => new Response(null, { status: 404 }));

    await expect(clipUrl("has-none")).resolves.toBeNull();
  });

  it("throws a failure rather than calling it no clip", async () => {
    answer(() => new Response(null, { status: 503 }));

    await expect(clipUrl("unanswerable")).rejects.toMatchObject({
      status: 503,
    });
  });
});
