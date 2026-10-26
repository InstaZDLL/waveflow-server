// The WCAG audit, the light theme, and contrast measured over an image.

import AxeBuilder from "@axe-core/playwright";
import { expect, type Locator, type Page } from "@playwright/test";

/**
 * Audit the page against WCAG A and AA, once it has stopped moving.
 *
 * The dark theme raises each block of a page into view, fading it in over
 * 640 ms. axe reads the colours as they are at the instant it looks, so an
 * audit that caught a block halfway measured its text at a fraction of its
 * opacity and reported a contrast the finished page does not have — on the
 * genre test, five runs in eight. Every finite animation is let finish first;
 * an infinite one, the pulse of a loading skeleton, would never settle, and is
 * left running.
 */
export async function expectNoWcagViolations(page: Page) {
  await page.evaluate(() =>
    Promise.all(
      document
        .getAnimations()
        .filter(
          (animation) =>
            animation.effect?.getComputedTiming().endTime !== Infinity,
        )
        .map((animation) => animation.finished.catch(() => undefined)),
    ),
  );
  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  expect(results.violations).toEqual([]);
}

/**
 * The light themes on the 2.0 structure. fluid.css lays the same rail, heads
 * and player bar over a light material, so every page the dark redesign
 * recomposed is audited again in the light: the home page has its own test
 * in `shell.spec.ts` ("keeps the light appearance readable"), and the tests
 * that call this follow the pages and the dialogs added since.
 */
export function chooseLightTheme(page: Page) {
  return page.addInitScript(() => {
    localStorage.setItem("waveflow.theme", "default");
  });
}

/**
 * The lowest contrast a text keeps over whatever is drawn behind it, pixel by
 * pixel. axe cannot measure text over a picture and reports it as needing
 * review, so the hero's legibility over a banner was never checked: the text
 * is made transparent, the box it sits in is captured, and its colour is
 * composited over every pixel of that capture. A pill is measured inside its
 * rounded ends and its ring, which are not behind the text: the corners of
 * its box show the picture it sits on.
 */
export async function lowestContrast(page: Page, target: Locator, pill = false) {
  const colour = await target.evaluate((element) => {
    const { color } = getComputedStyle(element);
    (element as HTMLElement).style.setProperty("color", "transparent", "important");
    return color;
  });
  const box = await target.boundingBox();
  if (!box) throw new Error("the text has no box to measure");
  const inset = pill ? { x: box.height / 2, y: 3 } : { x: 0, y: 0 };
  const shot = await page.screenshot({
    animations: "disabled",
    clip: {
      x: box.x + inset.x,
      y: box.y + inset.y,
      width: box.width - inset.x * 2,
      height: box.height - inset.y * 2,
    },
  });
  await target.evaluate((element) =>
    (element as HTMLElement).style.removeProperty("color"),
  );
  return page.evaluate(
    async ({ png, colour }) => {
      const image = new Image();
      image.src = `data:image/png;base64,${png}`;
      await image.decode();
      const canvas = document.createElement("canvas");
      canvas.width = image.width;
      canvas.height = image.height;
      const context = canvas.getContext("2d");
      if (!context) throw new Error("no 2D context");
      context.drawImage(image, 0, 0);
      const { data } = context.getImageData(0, 0, image.width, image.height);
      const [r, g, b, a = 1] = (colour.match(/[\d.]+/g) ?? []).map(Number);
      const channel = (value: number) => {
        const s = value / 255;
        return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
      };
      const luminance = (red: number, green: number, blue: number) =>
        0.2126 * channel(red) + 0.7152 * channel(green) + 0.0722 * channel(blue);
      let lowest = Number.POSITIVE_INFINITY;
      for (let index = 0; index < data.length; index += 4) {
        const [br, bg, bb] = [data[index], data[index + 1], data[index + 2]];
        const ground = luminance(br, bg, bb);
        const ink = luminance(
          a * r + (1 - a) * br,
          a * g + (1 - a) * bg,
          a * b + (1 - a) * bb,
        );
        const ratio =
          (Math.max(ground, ink) + 0.05) / (Math.min(ground, ink) + 0.05);
        lowest = Math.min(lowest, ratio);
      }
      return lowest;
    },
    { png: shot.toString("base64"), colour },
  );
}
