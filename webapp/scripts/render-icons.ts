/**
 * Renders the install icons in `public/` from the one mark (RFC-022 decision 4).
 *
 * Run by hand when the mark changes —
 * `node --experimental-strip-types scripts/render-icons.ts` from `webapp/` —
 * and commit what it writes. The flag is what lets Node 22.6 to 22.17 read
 * TypeScript; later versions strip types without it and still accept it. Not
 * under Bun: Playwright's browser launch hangs there.
 *
 * Not part of the build: the PNGs are assets like `favicon.svg`, and a build
 * that needed a browser to produce its icons would need one on every machine
 * that builds the server.
 *
 * The geometry is `logo.svg`'s, never redrawn: five bars, 140/80/40/80/140, in
 * a 256 box. Two framings:
 *
 * - **any**: the favicon's own, `favicon.svg` rendered larger — the rounded
 *   container in the gradient with the bars cut out of it in the application's
 *   dark. Its transparent corners are a launcher's to fill.
 * - **maskable** and the **apple-touch-icon**: full bleed, because a launcher
 *   or iOS cuts the shape itself, with the bars scaled into the safe zone — the
 *   central circle of 40% radius that every mask keeps. The bar group's
 *   corners sit at 45% of the box from its centre, so it is scaled by 0.72 to
 *   land at about 32%.
 */
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

import { chromium } from "@playwright/test";

const GRADIENT = `<linearGradient id="g" x1="0" y1="0" x2="256" y2="256" gradientUnits="userSpaceOnUse">
      <stop stop-color="#34D399"/>
      <stop offset="0.5" stop-color="#10B981"/>
      <stop offset="1" stop-color="#059669"/>
    </linearGradient>`;

const BARS = `<rect x="36" y="58" width="24" height="140" rx="12"/>
    <rect x="76" y="88" width="24" height="80" rx="12"/>
    <rect x="116" y="108" width="24" height="40" rx="12"/>
    <rect x="156" y="88" width="24" height="80" rx="12"/>
    <rect x="196" y="58" width="24" height="140" rx="12"/>`;

function fullBleed(scale: number): string {
  const offset = 128 * (1 - scale);
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
  <defs>${GRADIENT}</defs>
  <rect width="256" height="256" fill="url(#g)"/>
  <g fill="#101311" transform="translate(${offset} ${offset}) scale(${scale})">
    ${BARS}
  </g>
</svg>`;
}

const favicon = await readFile(
  new URL("../public/favicon.svg", import.meta.url),
  "utf8",
);

const outputs: Array<{ path: string; size: number; svg: string }> = [
  { path: "../public/icons/icon-192.png", size: 192, svg: favicon },
  { path: "../public/icons/icon-512.png", size: 512, svg: favicon },
  { path: "../public/icons/maskable-512.png", size: 512, svg: fullBleed(0.72) },
  { path: "../public/apple-touch-icon.png", size: 180, svg: fullBleed(0.72) },
];

const browser = await chromium.launch();
try {
  for (const { path, size, svg } of outputs) {
    const page = await browser.newPage({
      viewport: { width: size, height: size },
      deviceScaleFactor: 1,
    });
    const source = `data:image/svg+xml;base64,${Buffer.from(svg).toString("base64")}`;
    await page.setContent(
      `<html><body style="margin:0;background:transparent"><img src="${source}" width="${size}" height="${size}" style="display:block"></body></html>`,
    );
    await page.waitForFunction(() => document.images[0]?.complete === true);
    const target = new URL(path, import.meta.url);
    await page.screenshot({
      path: fileURLToPath(target),
      omitBackground: true,
    });
    await page.close();
    console.log(`wrote ${path} (${size}px)`);
  }
} finally {
  await browser.close();
}
