// WCAG contrast, measured from the tokens rather than from a screenshot.
//
// The presets write their accents as OKLCH and the stylesheets build every
// surface with `color-mix(in srgb, …)`, so asking "does this text hold AA on
// that ground" for fourteen presets needs the arithmetic the browser does:
// OKLCH to sRGB, the mix, alpha over a ground, then the WCAG 2 luminance. No
// dependency does that for us without pulling a colour library into a client
// that has none, and the whole of it is a page of maths.
//
// **Unclamped until the end.** The Tailwind v4 scales the accents come from
// sit partly outside sRGB (they were chosen in Display P3). A browser keeps an
// out-of-gamut value whole through `color-mix` and clips it only when it
// paints to an sRGB screen, so the channels stay unclamped here until the
// luminance, which clips each one the way that paint does.

/** A colour as gamma-encoded sRGB channels (nominally 0–1) and an alpha. */
export type Rgba = { r: number; g: number; b: number; alpha: number };

const TRANSPARENT: Rgba = { r: 0, g: 0, b: 0, alpha: 0 };

function linearToGamma(channel: number): number {
  const sign = channel < 0 ? -1 : 1;
  const magnitude = Math.abs(channel);
  return magnitude <= 0.0031308
    ? channel * 12.92
    : sign * (1.055 * magnitude ** (1 / 2.4) - 0.055);
}

function gammaToLinear(channel: number): number {
  return channel <= 0.04045
    ? channel / 12.92
    : ((channel + 0.055) / 1.055) ** 2.4;
}

/**
 * OKLCH to gamma-encoded sRGB, through OKLab and linear sRGB with Björn
 * Ottosson's published matrices (the ones CSS Color 4 specifies). The result
 * is not clamped: see the note at the head of this file.
 */
export function oklchToSrgb(
  lightness: number,
  chroma: number,
  hueDegrees: number,
): [number, number, number] {
  const hue = (hueDegrees * Math.PI) / 180;
  const a = chroma * Math.cos(hue);
  const b = chroma * Math.sin(hue);
  const l = (lightness + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (lightness - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (lightness - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    linearToGamma(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
    linearToGamma(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
    linearToGamma(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s),
  ];
}

function parseAlpha(text: string | undefined): number {
  if (text === undefined) return 1;
  const value = text.trim();
  return value.endsWith("%") ? Number.parseFloat(value) / 100 : Number(value);
}

/**
 * One colour literal as the stylesheets write them: `#rgb`, `#rrggbb`,
 * `rgb(r g b / a)`, `oklch(l c h)`, `white`, `black` or `transparent`.
 * Anything else throws — a measurement that silently read an unknown colour
 * as black would pass for the wrong reason.
 */
export function parseColor(text: string): Rgba {
  const value = text.trim().toLowerCase();
  if (value === "transparent") return TRANSPARENT;
  if (value === "white") return { r: 1, g: 1, b: 1, alpha: 1 };
  if (value === "black") return { r: 0, g: 0, b: 0, alpha: 1 };
  const hex = value.match(/^#([0-9a-f]{3}|[0-9a-f]{6})$/);
  if (hex?.[1]) {
    const digits =
      hex[1].length === 3
        ? [...hex[1]].map((digit) => digit + digit).join("")
        : hex[1];
    const channel = (index: number) =>
      Number.parseInt(digits.slice(index * 2, index * 2 + 2), 16) / 255;
    return { r: channel(0), g: channel(1), b: channel(2), alpha: 1 };
  }
  const rgb = value.match(
    /^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)\s*(?:[/,]\s*([\d.]+%?))?\s*\)$/,
  );
  if (rgb) {
    return {
      r: Number(rgb[1]) / 255,
      g: Number(rgb[2]) / 255,
      b: Number(rgb[3]) / 255,
      alpha: parseAlpha(rgb[4]),
    };
  }
  const oklch = value.match(
    /^oklch\(\s*([\d.]+)(%?)\s+([\d.]+)\s+([\d.]+)\s*(?:\/\s*([\d.]+%?))?\s*\)$/,
  );
  if (oklch) {
    const lightness = Number(oklch[1]) / (oklch[2] === "%" ? 100 : 1);
    const [r, g, b] = oklchToSrgb(
      lightness,
      Number(oklch[3]),
      Number(oklch[4]),
    );
    return { r, g, b, alpha: parseAlpha(oklch[5]) };
  }
  throw new Error(`contrast: unsupported colour "${text}"`);
}

/**
 * `color-mix(in srgb, first p1, second p2)` as CSS Color 5 defines it: a
 * missing percentage is the other's complement, both are scaled to a sum of
 * 100, a sum below 100 lowers the alpha by as much, and the channels are
 * interpolated premultiplied by their alpha — which is why mixing a colour
 * with `transparent` makes it translucent rather than darker.
 */
export function mixSrgb(
  first: Rgba,
  firstPercent: number | null,
  second: Rgba,
  secondPercent: number | null,
): Rgba {
  let p1 = firstPercent;
  let p2 = secondPercent;
  if (p1 === null && p2 === null) {
    p1 = 50;
    p2 = 50;
  } else if (p1 === null) {
    p1 = 100 - (p2 as number);
  } else if (p2 === null) {
    p2 = 100 - p1;
  }
  const sum = (p1 as number) + (p2 as number);
  const w1 = (p1 as number) / sum;
  const w2 = (p2 as number) / sum;
  const alpha = first.alpha * w1 + second.alpha * w2;
  const multiplier = Math.min(sum, 100) / 100;
  if (alpha === 0) return { ...TRANSPARENT };
  const channel = (key: "r" | "g" | "b") =>
    (first[key] * first.alpha * w1 + second[key] * second.alpha * w2) / alpha;
  return {
    r: channel("r"),
    g: channel("g"),
    b: channel("b"),
    alpha: alpha * multiplier,
  };
}

/** `top` painted over `bottom`, which is taken as opaque. */
export function composite(top: Rgba, bottom: Rgba): Rgba {
  const a = top.alpha;
  return {
    r: top.r * a + bottom.r * (1 - a),
    g: top.g * a + bottom.g * (1 - a),
    b: top.b * a + bottom.b * (1 - a),
    alpha: 1,
  };
}

/** WCAG 2 relative luminance, each channel clipped to what a screen shows. */
export function relativeLuminance(colour: Rgba): number {
  const clip = (channel: number) => Math.min(1, Math.max(0, channel));
  return (
    0.2126 * gammaToLinear(clip(colour.r)) +
    0.7152 * gammaToLinear(clip(colour.g)) +
    0.0722 * gammaToLinear(clip(colour.b))
  );
}

/**
 * The WCAG contrast ratio of `foreground` on `background`. A translucent
 * foreground is first painted over the background, as it would be on screen;
 * the background must already be opaque (composite it over its own ground).
 */
export function contrastRatio(foreground: Rgba, background: Rgba): number {
  if (background.alpha < 1) {
    throw new Error("contrast: the background must be opaque");
  }
  const ink =
    foreground.alpha < 1 ? composite(foreground, background) : foreground;
  const a = relativeLuminance(ink);
  const b = relativeLuminance(background);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

/** A colour as `#rrggbb`, clipped — for messages and reference tests. */
export function toHex(colour: Rgba): string {
  const byte = (channel: number) =>
    Math.round(Math.min(1, Math.max(0, channel)) * 255)
      .toString(16)
      .padStart(2, "0");
  return `#${byte(colour.r)}${byte(colour.g)}${byte(colour.b)}`;
}
