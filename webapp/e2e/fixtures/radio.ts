// A station and a queue served together, and a live station's elements.

import type { Page } from "@playwright/test";

import { silentWav } from "./media";
import { mock } from "./state";

/**
 * Moves within the client, as a click on its navigation does — never by
 * loading a page, which would start the player over and prove nothing about
 * what survives a navigation.
 */
export async function navigateWithin(page: Page, name: string) {
  const sidebar = page.locator(".sidebar");
  if (await sidebar.isVisible()) {
    await sidebar.getByRole("link", { name, exact: true }).click();
    return;
  }
  const bar = page.locator(".mobile-navigation");
  const direct = bar.getByRole("link", { name, exact: true });
  if (await direct.count()) {
    await direct.click();
    return;
  }
  await bar.getByRole("button", { name: "More" }).click();
  await page.getByRole("dialog").getByRole("link", { name, exact: true }).click();
}

export const NOVA = "https://nova.example.org/live";

/**
 * A station on another origin that plays, a queue whose tracks play, and a
 * record of every element asked to play and every element the ReplayGain
 * graph claimed — none of them is in the document, so this is the only way
 * to reach them. ReplayGain is on, so the graph exists to claim something.
 */
export async function serveStationAndQueue(page: Page) {
  await page.addInitScript(() => {
    localStorage.setItem(
      "waveflow.replayGain",
      JSON.stringify({ mode: "album", preampDb: 0 }),
    );
    const record = window as unknown as {
      __played: Set<HTMLMediaElement>;
      __claimed: Set<HTMLMediaElement>;
    };
    record.__played = new Set();
    record.__claimed = new Set();
    const play = HTMLMediaElement.prototype.play;
    HTMLMediaElement.prototype.play = function () {
      record.__played.add(this);
      return play.call(this);
    };
    const claim = AudioContext.prototype.createMediaElementSource;
    AudioContext.prototype.createMediaElementSource = function (element) {
      record.__claimed.add(element);
      return claim.call(this, element);
    };
  });
  mock.stations = [
    {
      id: "station-1",
      name: "Radio Nova",
      stream_url: NOVA,
      homepage_url: null,
      image_url: null,
      artwork_hash: null,
      created_at: 1_700_000_000_000,
      updated_at: 1_700_000_000_000,
    },
  ];
  const audio = silentWav(60);
  // Served with no CORS header at all, as most stations are: an element plays
  // it, and a MediaElementSource would turn it to silence.
  await page.route(`${NOVA}**`, (route) =>
    route.fulfill({ body: audio, headers: { "content-type": "audio/wav" } }),
  );
  await page.route("**/api/v2/tracks/*/stream-ticket", (route) => {
    const trackId = new URL(route.request().url()).pathname.split("/")[4];
    return route.fulfill({
      json: {
        url: `/api/v2/stream/ticket-${trackId}`,
        expires_at: Date.now() + 60_000,
      },
    });
  });
  await page.route("**/api/v2/stream/*", (route) =>
    route.fulfill({ body: audio, headers: { "content-type": "audio/wav" } }),
  );
}

/** What the station's element and the queue's are doing, as seen now. */
export function elements(page: Page) {
  return page.evaluate((nova) => {
    const record = window as unknown as {
      __played: Set<HTMLMediaElement>;
      __claimed: Set<HTMLMediaElement>;
      __station?: HTMLMediaElement;
    };
    const played = [...record.__played];
    // Found by its source while it has one, and kept: stopping a station lets
    // go of its source, and the element must still be found afterwards.
    record.__station ??= played.find((element) => element.currentSrc === nova);
    const station = record.__station;
    const queue = played.find((element) =>
      element.currentSrc.includes("/api/v2/stream/ticket-"),
    );
    return {
      station: station
        ? {
            paused: station.paused,
            time: station.currentTime,
            source: station.currentSrc,
            claimed: record.__claimed.has(station),
          }
        : null,
      queue: queue
        ? {
            paused: queue.paused,
            source: queue.currentSrc,
            claimed: record.__claimed.has(queue),
          }
        : null,
      claims: record.__claimed.size,
    };
  }, NOVA);
}
