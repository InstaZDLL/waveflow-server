import { describe, expect, it } from "vitest";

import { creditedArtists } from "./artist-credits";

describe("creditedArtists", () => {
  it("uses the structured list, in tag order, when the server sends one", () => {
    expect(
      creditedArtists({
        artist: "Bleu Soleil, LUIZA",
        artist_id: "artist-1",
        artists: [
          { id: "artist-1", name: "Bleu Soleil" },
          { id: "artist-2", name: "LUIZA" },
        ],
      }),
    ).toEqual([
      { id: "artist-1", name: "Bleu Soleil" },
      { id: "artist-2", name: "LUIZA" },
    ]);
  });

  it("falls back to the primary credit when the list is missing or empty", () => {
    const primary = [{ id: "artist-1", name: "Björk" }];
    expect(creditedArtists({ artist: "Björk", artist_id: "artist-1" })).toEqual(
      primary,
    );
    expect(
      creditedArtists({ artist: "Björk", artist_id: "artist-1", artists: [] }),
    ).toEqual(primary);
  });

  it("links nothing for an artist the catalogue does not know", () => {
    expect(
      creditedArtists({
        artist: "Nobody We Know",
        artist_id: null,
        artists: [],
      }),
    ).toEqual([]);
    expect(creditedArtists({ artist: null, artist_id: null })).toEqual([]);
  });
});
