/**
 * The cover's light: the colour the Fluid Editorial direction spreads behind
 * the page, read off the playing record's sleeve.
 *
 * Ported from the desktop's `dominantColor` and `useCoverGlow`, so the two
 * clients light a page the same way from the same cover. No React here: the
 * pixel arithmetic is what can be wrong, and it is tested on its own.
 */

export type Rgb = { r: number; g: number; b: number };

/**
 * Average the pixels that carry the cover's feel.
 *
 * Every fourth pixel of the sample, skipping the transparent ones and the
 * near-monochrome extremes — a white margin or a black bar would otherwise
 * flatten every average towards grey. `null` when nothing is left, so the
 * caller keeps the light it already has rather than inventing one.
 */
export function averageColour(data: ArrayLike<number>): Rgb | null {
  let r = 0;
  let g = 0;
  let b = 0;
  let count = 0;
  for (let i = 0; i + 3 < data.length; i += 16) {
    const pr = data[i];
    const pg = data[i + 1];
    const pb = data[i + 2];
    if (data[i + 3] < 128) continue;
    const max = Math.max(pr, pg, pb);
    const min = Math.min(pr, pg, pb);
    if (max - min < 24 && (max < 32 || max > 224)) continue;
    r += pr;
    g += pg;
    b += pb;
    count += 1;
  }
  if (count === 0) return null;
  return {
    r: Math.round(r / count),
    g: Math.round(g / count),
    b: Math.round(b / count),
  };
}

/**
 * Scale a colour towards black, harder for a bright cover and softer for a
 * dark one, so a white sleeve never pales the ground and a black one still
 * shows. The factors are the desktop's.
 */
export function shade({ r, g, b }: Rgb, k: number): string {
  const lum = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
  const factor = lum > 0.6 ? k * 0.55 : lum < 0.12 ? k * 1.8 : k;
  const channel = (v: number) =>
    Math.max(0, Math.min(255, Math.round(v * factor)))
      .toString(16)
      .padStart(2, "0");
  return `#${channel(r)}${channel(g)}${channel(b)}`;
}

/** The two pools: the cover's tone deepened, and the same again, darker. */
export function glowOf(colour: Rgb): { glow: string; deep: string } {
  return { glow: shade(colour, 0.62), deep: shade(colour, 0.3) };
}

const SAMPLE = 64;

/**
 * Read a cover's average colour from an image URL.
 *
 * The URL is an object URL this client made from bytes it fetched itself, so
 * the canvas is never tainted and no CORS question arises.
 */
export function sampleCover(url: string): Promise<Rgb | null> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => {
      const canvas = document.createElement("canvas");
      canvas.width = SAMPLE;
      canvas.height = SAMPLE;
      const context = canvas.getContext("2d", { willReadFrequently: true });
      if (!context) {
        resolve(null);
        return;
      }
      context.drawImage(image, 0, 0, SAMPLE, SAMPLE);
      resolve(averageColour(context.getImageData(0, 0, SAMPLE, SAMPLE).data));
    };
    image.onerror = () => reject(new Error("cover could not be decoded"));
    image.src = url;
  });
}
