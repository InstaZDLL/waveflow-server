import { describe, expect, it } from "vitest";

import { lyricSheetState, playingView } from "./playing-page";

/**
 * Which view a location asks for.
 *
 * This reads a value off the URL, so it is the one place the playing screen
 * trusts something a stranger can write — and it is the **only** narrowing:
 * the route carried a `validateSearch` doing the same job, which removing
 * changed no test and no behaviour, because `useSearch` reads the raw search
 * and this is what decides.
 */
describe("playingView", () => {
  it("reads the two modes that are spelled", () => {
    expect(playingView({ view: "lyrics" })).toBe("lyrics");
    expect(playingView({ view: "immersive" })).toBe("immersive");
  });

  it("treats an absent view as the detail view", () => {
    expect(playingView({})).toBe("detail");
  });

  it("accepts the default spelled out, without keeping it", () => {
    // `/playing?view=detail` is not a URL this client writes — the route
    // returns `{}` for it — but it is one somebody can type, and it means the
    // detail view rather than nothing.
    expect(playingView({ view: "detail" })).toBe("detail");
  });

  it("reads anything it does not know as the detail view", () => {
    // A mode from a later build, a hostile string, a value of the wrong type:
    // an unknown mode is not a mode, and the screen still has to show
    // something.
    for (const view of [
      "immersive ",
      "IMMERSIVE",
      "<script>alert(1)</script>",
      "__proto__",
      "",
      0,
      1,
      true,
      null,
      undefined,
      ["lyrics"],
      { view: "lyrics" },
    ]) {
      expect(playingView({ view })).toBe("detail");
    }
  });

  it("reads a search that is not an object as the detail view", () => {
    for (const search of [undefined, null, "view=lyrics", 7, [], true]) {
      expect(playingView(search)).toBe("detail");
    }
  });
});

/**
 * What the lyrics panel says, from what the query holds.
 *
 * The case that matters is the third one: the cache serves an answer for thirty
 * seconds, so returning to this view revalidates, and a revalidation that fails
 * used to print "no lyrics travel with this file" over a sheet that was on the
 * screen a moment earlier. That is a false statement about the track, and it is
 * the same mistake as reading a failed request as an empty list.
 */
describe("lyricSheetState", () => {
  it("waits while nothing has come back", () => {
    expect(lyricSheetState({ answered: false, failed: false, lines: 0 })).toBe(
      "waiting",
    );
  });

  it("says a track has no words when the first attempt fails", () => {
    expect(lyricSheetState({ answered: false, failed: true, lines: 0 })).toBe(
      "none",
    );
  });

  it("keeps words it already holds through a failed revalidation", () => {
    expect(lyricSheetState({ answered: true, failed: true, lines: 3 })).toBe(
      "sheet",
    );
  });

  it("says a track has no words when the answer carries none", () => {
    // An answered query with an empty sheet is the server saying so, which is
    // a different thing from a request that did not arrive.
    expect(lyricSheetState({ answered: true, failed: false, lines: 0 })).toBe(
      "none",
    );
    expect(lyricSheetState({ answered: true, failed: true, lines: 0 })).toBe(
      "none",
    );
  });

  it("shows the sheet it was given", () => {
    expect(lyricSheetState({ answered: true, failed: false, lines: 1 })).toBe(
      "sheet",
    );
  });
});
