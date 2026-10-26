import { describe, expect, it } from "vitest";

import { placeMenu } from "./menu-place";

const viewport = { width: 1000, height: 600 };
const menu = { width: 200, height: 120 };

describe("placeMenu", () => {
  it("sets a menu at its point, by its left or its right edge", () => {
    expect(
      placeMenu({ ...menu, x: 100, y: 50, align: "start", viewport }),
    ).toEqual({ left: 100, top: 50 });
    expect(
      placeMenu({ ...menu, x: 500, y: 50, align: "end", viewport }),
    ).toEqual({ left: 300, top: 50 });
  });

  it("keeps a menu inside the window at every edge", () => {
    expect(
      placeMenu({ ...menu, x: 950, y: 590, align: "start", viewport }),
    ).toEqual({ left: 792, top: 472 });
    expect(
      placeMenu({ ...menu, x: 50, y: -40, align: "end", viewport }),
    ).toEqual({ left: 8, top: 8 });
  });

  // A button near the bottom of the window: the menu hangs above it rather
  // than being pushed up over it.
  it("hangs a menu above its button when there is no room below", () => {
    expect(
      placeMenu({
        ...menu,
        x: 500,
        y: 540,
        above: 500,
        align: "end",
        viewport,
      }),
    ).toEqual({ left: 300, top: 380 });
  });

  it("leaves a menu below its button when it fits there", () => {
    expect(
      placeMenu({
        ...menu,
        x: 500,
        y: 300,
        above: 260,
        align: "end",
        viewport,
      }),
    ).toEqual({ left: 300, top: 300 });
  });
});
