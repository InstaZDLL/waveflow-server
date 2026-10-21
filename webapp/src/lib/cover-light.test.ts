import { describe, expect, it } from "vitest";

import { averageColour, glowOf, shade } from "./cover-light";

/** `count` sampled pixels of one colour, laid out the way the walk reads them. */
function pixels(colour: [number, number, number, number], count: number) {
  const data: number[] = [];
  for (let n = 0; n < count; n += 1) {
    // The walk reads one pixel in four, so three unread ones follow each.
    data.push(...colour, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0);
  }
  return data;
}

describe("the cover's average colour", () => {
  it("averages the colourful pixels", () => {
    const data = [
      ...pixels([200, 40, 40, 255], 3),
      ...pixels([40, 40, 200, 255], 1),
    ];
    expect(averageColour(data)).toEqual({ r: 160, g: 40, b: 80 });
  });

  it("leaves out the margins, the bars and the transparent pixels", () => {
    // Without the skip, the white and the black would drag this to grey.
    const data = [
      ...pixels([200, 40, 40, 255], 1),
      ...pixels([250, 250, 250, 255], 5),
      ...pixels([5, 5, 5, 255], 5),
      ...pixels([40, 200, 40, 10], 5),
    ];
    expect(averageColour(data)).toEqual({ r: 200, g: 40, b: 40 });
  });

  it("has nothing to say about a cover with nothing left", () => {
    expect(averageColour(pixels([255, 255, 255, 255], 4))).toBeNull();
    expect(averageColour([])).toBeNull();
  });
});

describe("the light it becomes", () => {
  it("deepens a mid-tone cover by the factor asked", () => {
    expect(shade({ r: 100, g: 100, b: 200 }, 0.5)).toBe("#323264");
  });

  it("pulls a bright cover down harder than a mid-tone one", () => {
    // Luminance above 0.6 takes 55% of the factor: a pale sleeve must never
    // pale the ground.
    expect(shade({ r: 240, g: 240, b: 240 }, 0.5)).toBe("#424242");
  });

  it("lifts a very dark cover so it still shows", () => {
    expect(shade({ r: 20, g: 10, b: 10 }, 0.5)).toBe("#120909");
  });

  it("makes the deep pool darker than the near one", () => {
    const { glow, deep } = glowOf({ r: 52, g: 211, b: 153 });
    expect(Number.parseInt(deep.slice(1, 3), 16)).toBeLessThan(
      Number.parseInt(glow.slice(1, 3), 16),
    );
  });
});
