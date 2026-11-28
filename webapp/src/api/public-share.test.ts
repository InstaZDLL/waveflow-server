import { afterEach, describe, expect, it, vi } from "vitest";

import { forgetServerBase } from "./base";
import { openPublicShare } from "./public-share";

function servedUnder(prefix: string) {
  for (const meta of document.querySelectorAll('meta[name="waveflow-base"]')) {
    meta.remove();
  }
  const meta = document.createElement("meta");
  meta.name = "waveflow-base";
  meta.content = prefix;
  document.head.append(meta);
  forgetServerBase();
}

/** A share as a server with no public URL answers it: root paths. */
const answered = {
  id: "share-1",
  description: null,
  expiresAt: null,
  visitCount: 1,
  tracks: [
    {
      id: "track-1",
      title: "Hidden Place",
      streamUrl: "/share/tok/tracks/track-1/stream?visit=v",
      coverUrl: "/share/tok/tracks/track-1/cover?visit=v",
    },
    {
      id: "track-2",
      title: "Cocoon",
      streamUrl: "https://music.example.org/share/tok/tracks/track-2/stream",
    },
  ],
};

afterEach(() => {
  servedUnder("");
  vi.unstubAllGlobals();
});

describe("openPublicShare", () => {
  it("asks for a share's stream and cover under Home Assistant's prefix (RFC-030)", async () => {
    servedUnder("/api/hassio_ingress/tok");
    const fetched = vi.fn(async () => Response.json(answered));
    vi.stubGlobal("fetch", fetched);

    const share = await openPublicShare("tok");

    expect(fetched).toHaveBeenCalledWith(
      "/api/hassio_ingress/tok/share/tok",
      expect.objectContaining({ credentials: "same-origin" }),
    );
    expect(share?.tracks[0].streamUrl).toBe(
      "/api/hassio_ingress/tok/share/tok/tracks/track-1/stream?visit=v",
    );
    expect(share?.tracks[0].coverUrl).toBe(
      "/api/hassio_ingress/tok/share/tok/tracks/track-1/cover?visit=v",
    );
    // A full address, as a server with a public URL answers, goes through.
    expect(share?.tracks[1].streamUrl).toBe(
      "https://music.example.org/share/tok/tracks/track-2/stream",
    );
    expect(share?.tracks[1]).not.toHaveProperty("coverUrl");
  });

  it("leaves them as answered at the root", async () => {
    servedUnder("");
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => Response.json(answered)),
    );
    const share = await openPublicShare("tok");
    expect(share?.tracks[0].streamUrl).toBe(answered.tracks[0].streamUrl);
  });
});
