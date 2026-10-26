// The colour maths the per-preset contrast test stands on, held to values
// published elsewhere: a conversion that drifted would move every ratio the
// other test reads, and a ratio is only as good as the colour under it.

import { describe, expect, it } from "vitest";

import {
  composite,
  contrastRatio,
  mixSrgb,
  oklchToSrgb,
  parseColor,
  toHex,
} from "./contrast";

describe("oklchToSrgb", () => {
  it("lands the sRGB primaries on their published OKLCH coordinates", () => {
    // The primaries in OKLCH, as CSS Color 4's sample code and oklch.com
    // give them.
    const cases: Array<[number, number, number, [number, number, number]]> = [
      [0.627955, 0.257683, 29.2339, [1, 0, 0]],
      [0.86644, 0.294827, 142.4953, [0, 1, 0]],
      [0.452014, 0.313214, 264.052, [0, 0, 1]],
      [1, 0, 0, [1, 1, 1]],
      [0, 0, 0, [0, 0, 0]],
    ];
    for (const [l, c, h, expected] of cases) {
      const actual = oklchToSrgb(l, c, h);
      for (let index = 0; index < 3; index += 1) {
        expect(actual[index], `oklch(${l} ${c} ${h})[${index}]`).toBeCloseTo(
          expected[index] as number,
          3,
        );
      }
    }
  });

  it("matches the hex Tailwind v4 publishes for its in-gamut stops", () => {
    // Tailwind's own sRGB fallbacks for the stops the light material spends.
    expect(toHex(parseColor("oklch(0.508 0.118 165.612)"))).toBe("#007a55");
    expect(toHex(parseColor("oklch(0.5 0.134 242.749)"))).toBe("#0069a8");
    expect(toHex(parseColor("oklch(0.457 0.24 277.023)"))).toBe("#432dd7");
  });

  it("keeps an out-of-gamut colour whole until the end", () => {
    // Emerald 500 is a Display P3 colour: its red channel is below 0 in
    // sRGB, and only the paint clips it.
    const [r] = oklchToSrgb(0.696, 0.17, 162.48);
    expect(r).toBeLessThan(-0.2);
  });
});

describe("parseColor", () => {
  it("reads the spellings the stylesheets use", () => {
    expect(toHex(parseColor("#abc"))).toBe("#aabbcc");
    expect(toHex(parseColor("#1b1a17"))).toBe("#1b1a17");
    expect(parseColor("rgb(27 26 23 / 9%)").alpha).toBeCloseTo(0.09, 6);
    expect(parseColor("rgb(255 255 255 / 0.5)").alpha).toBe(0.5);
    expect(parseColor("transparent").alpha).toBe(0);
    expect(toHex(parseColor("white"))).toBe("#ffffff");
  });

  it("refuses what it cannot read rather than guessing", () => {
    expect(() => parseColor("hsl(0 0% 50%)")).toThrow(/unsupported/);
  });
});

describe("mixSrgb", () => {
  it("mixes in gamma-encoded sRGB, as color-mix(in srgb) does", () => {
    // Halfway between black and white in sRGB is #808080, not the
    // perceptual or the linear-light middle.
    const half = mixSrgb(parseColor("white"), 50, parseColor("black"), null);
    expect(toHex(half)).toBe("#808080");
  });

  it("turns a mix with transparent into translucency, not darkness", () => {
    const soft = mixSrgb(
      parseColor("#ff0000"),
      10,
      parseColor("transparent"),
      null,
    );
    expect(soft.alpha).toBeCloseTo(0.1, 6);
    expect(toHex(soft)).toBe("#ff0000");
  });

  it("lowers the alpha when the percentages sum below 100", () => {
    const thin = mixSrgb(parseColor("#000"), 20, parseColor("#fff"), 20);
    expect(thin.alpha).toBeCloseTo(0.4, 6);
    expect(toHex(thin)).toBe("#808080");
  });
});

describe("contrastRatio", () => {
  it("gives WCAG's own reference ratios", () => {
    expect(contrastRatio(parseColor("#fff"), parseColor("#000"))).toBeCloseTo(
      21,
      5,
    );
    expect(contrastRatio(parseColor("#000"), parseColor("#000"))).toBe(1);
    // #767676 is the lightest grey that holds 4.5:1 on white; #777 is not.
    expect(
      contrastRatio(parseColor("#767676"), parseColor("#fff")),
    ).toBeCloseTo(4.54, 2);
    expect(contrastRatio(parseColor("#777"), parseColor("#fff"))).toBeLessThan(
      4.5,
    );
  });

  it("paints a translucent foreground over its ground first", () => {
    const veil = parseColor("rgb(0 0 0 / 50%)");
    const white = parseColor("#fff");
    expect(contrastRatio(veil, white)).toBeCloseTo(
      contrastRatio(composite(veil, white), white),
      10,
    );
  });

  it("refuses a translucent ground, which has no colour of its own", () => {
    expect(() =>
      contrastRatio(parseColor("#fff"), parseColor("rgb(0 0 0 / 50%)")),
    ).toThrow(/opaque/);
  });
});
