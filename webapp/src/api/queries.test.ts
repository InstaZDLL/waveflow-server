import { describe, expect, it } from "vitest";

import type { ArtistDetail, ArtistEnrichment } from "./index";
import {
  ENRICHMENT_ASKS,
  ENRICHMENT_RETRY_MS,
  enrichmentRetryDelay,
} from "./queries";

const artist = (state?: ArtistEnrichment["state"]): ArtistDetail => ({
  id: "artist-1",
  library_id: "library-1",
  name: "Björk",
  album_count: 0,
  artwork_hash: null,
  albums: [],
  enrichment: state
    ? { state, bio: null, image: null, similar: null }
    : undefined,
});

/**
 * The web client learns that a provider answered only by asking again, so a
 * pending artist is asked again — and a provider that never answers must not
 * keep an open page asking forever.
 */
describe("enrichmentRetryDelay", () => {
  it("asks again while pending, then stops at the bound", () => {
    expect(enrichmentRetryDelay(artist("pending"), 1)).toBe(
      ENRICHMENT_RETRY_MS,
    );
    expect(enrichmentRetryDelay(artist("pending"), ENRICHMENT_ASKS - 1)).toBe(
      ENRICHMENT_RETRY_MS,
    );
    expect(enrichmentRetryDelay(artist("pending"), ENRICHMENT_ASKS)).toBe(
      false,
    );
  });

  it("never asks again once there is an answer, or nothing to wait for", () => {
    for (const state of ["ready", "none", "disabled"] as const) {
      expect(enrichmentRetryDelay(artist(state), 1)).toBe(false);
    }
    expect(enrichmentRetryDelay(artist(), 1)).toBe(false);
    expect(enrichmentRetryDelay(undefined, 0)).toBe(false);
  });
});
