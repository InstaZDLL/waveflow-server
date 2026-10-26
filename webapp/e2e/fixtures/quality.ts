// A library's measured quality and its analysis, as the page is served.

import type { Page } from "@playwright/test";

import { song } from "./catalogue";

/**
 * The quality page (RFC-016), as the server answers it: the counts per
 * verdict, the files worst first, one file's measurement with its averaged
 * spectrum, and a spectrogram drawn when asked. Held here so a test can watch
 * what the page asks for and writes.
 */
export async function serveQuality(page: Page) {
  const fake = {
    ...song(31, "Made From An MP3", 0, false),
    bit_depth: 24,
    bitrate: 812,
    quality: {
      verdict: "lossy_source",
      cutoff_hz: 16_390,
      wall_db: 41.2,
      nyquist_hz: 22_050,
      effective_bits: 24,
      analyzed_at: 1_700_000_000_000,
    },
  };
  const genuine = {
    ...song(32, "Straight From The Desk", 0, false),
    quality: {
      verdict: "consistent",
      cutoff_hz: null,
      wall_db: 4.1,
      nyquist_hz: 22_050,
      effective_bits: 16,
      analyzed_at: 1_700_000_000_000,
    },
  };
  const state = {
    asked: [] as string[],
    writes: [] as string[],
    spectrograms: 0,
    summaries: 0,
    /** What the summary says, one entry per ask; the last one repeats. */
    analyses: [
      {
        measured: 2,
        analysis: {
          state: "idle",
          in_flight: 0,
          pass_started_at: null,
          last_measured_at: 1_700_000_000_000,
        },
      },
    ] as Array<{ measured: number; analysis: Record<string, unknown> }>,
  };
  const base = "**/api/v2/libraries/library-1";
  await page.route(`${base}/quality`, (route) => {
    const now =
      state.analyses[Math.min(state.summaries, state.analyses.length - 1)];
    state.summaries += 1;
    return route.fulfill({
      json: {
        enabled: true,
        lossless: 3,
        measured: now?.measured ?? 2,
        analysis: now?.analysis,
        verdicts: [
          { verdict: "lossy_source", count: 1 },
          { verdict: "upsampled", count: 0 },
          { verdict: "padded", count: 0 },
          { verdict: "unreadable", count: 0 },
          { verdict: "silent", count: 0 },
          { verdict: "pending", count: 1 },
          { verdict: "consistent", count: 1 },
          { verdict: "lossy", count: 4 },
          { verdict: "not_measured", count: 0 },
        ],
      },
    });
  });
  await page.route(`${base}/quality/tracks?*`, (route) => {
    const url = new URL(route.request().url());
    state.asked.push(url.search);
    const verdict = url.searchParams.get("verdict");
    const all = [fake, genuine];
    return route.fulfill({
      json: verdict
        ? all.filter((item) => item.quality.verdict === verdict)
        : all,
    });
  });
  await page.route(`${base}/quality/analysis`, async (route) => {
    state.writes.push(route.request().method());
    await route.fulfill({ status: 202, body: "" });
  });
  await page.route("**/api/v2/tracks/song-31/quality", (route) =>
    route.fulfill({
      json: {
        ...fake.quality,
        // Level to 16 kHz, then a wall: what LAME leaves at 128 kbps.
        spectrum: Array.from({ length: 256 }, (_, band) =>
          band < 190 ? -20 - band / 20 : -95,
        ),
      },
    }),
  );
  await page.route("**/api/v2/tracks/song-31", (route) =>
    route.fulfill({
      json: {
        ...fake,
        relative_path: "Fakes/Made From An MP3.flac",
        full_hash: "b".repeat(64),
        created_at: 1_700_000_000_000,
      },
    }),
  );
  // No correction: the catch-all would answer with a track, which the panel
  // would read as one.
  await page.route("**/api/v2/tracks/song-31/overrides", (route) =>
    route.fulfill({ status: 404, json: { error: "not found" } }),
  );
  await page.route("**/api/v2/tracks/song-31/spectrogram", async (route) => {
    state.spectrograms += 1;
    await route.fulfill({
      status: 200,
      headers: {
        "content-type": "image/png",
        "cache-control": "private, no-cache",
      },
      body: Buffer.from(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z4m8AAAAASUVORK5CYII=",
        "base64",
      ),
    });
  });
  return state;
}

/** What the summary says of the analysis, at rest or at work. */
export function analysisAt(
  measured: number,
  state: "idle" | "running",
): { measured: number; analysis: Record<string, unknown> } {
  return {
    measured,
    analysis: {
      state,
      in_flight: state === "running" ? 1 : 0,
      pass_started_at: state === "running" ? 1_700_000_000_000 : null,
      last_measured_at: 1_700_000_000_000,
    },
  };
}
