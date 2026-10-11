import { describe, expect, it } from "vitest";

import { playingView } from "./playing-page";

/**
 * Which view a location asks for.
 *
 * This reads a value off the URL, so it is the one place the playing screen
 * trusts something a stranger can write. It is also what `validateSearch` in
 * `main.tsx` calls, so a mode it lets through reaches both the component and
 * the address bar.
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
