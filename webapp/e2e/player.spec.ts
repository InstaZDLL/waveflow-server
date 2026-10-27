// The player: its bar, the queue, ReplayGain and the audio element.

import { expect, test } from "@playwright/test";

import { expectNoWcagViolations } from "./fixtures/accessibility";
import { resetAndMockApi } from "./fixtures/api";
import { albums, track } from "./fixtures/catalogue";
import { silentWav } from "./fixtures/media";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

/**
 * The heard part of the line ends under the thumb, wherever the track is.
 *
 * The wave is placed in the progress grid's middle column. Named by its start
 * alone, an absolutely placed item runs to the container's edge, so the wave
 * was measured against the gap and the duration's column as well and outran
 * the thumb further the later the track. Driven through `--played`, the value
 * the player writes, so every position is reached without playing anything.
 */
test("ends the played wave under the progress thumb", async ({
  page,
}, testInfo) => {
  test.skip(
    testInfo.project.name !== "desktop",
    "the wave is drawn beside the rail",
  );
  await page.goto("/");
  const progress = page.locator(".player-progress");
  await expect(progress).toBeVisible();
  const measured = await progress.evaluate((node: HTMLElement) => {
    const wave = node.querySelector(".progress-wave") as HTMLElement;
    const input = node.querySelector("input") as HTMLInputElement;
    // The thumb's size as the stylesheet declares it, in rem — and the size it
    // has always had where no token names it, so a stylesheet without one is
    // judged on where its wave ends rather than on a missing variable.
    const rem =
      Number.parseFloat(
        getComputedStyle(node).getPropertyValue("--fe-thumb"),
      ) || 0.625;
    const thumb =
      rem *
      Number.parseFloat(getComputedStyle(document.documentElement).fontSize);
    // The range's own bounds. A mocked track may carry no duration, which
    // leaves `max` at zero and every value clamped to it, so the range is
    // given a span to move along when it has none.
    const min = Number(input.min) || 0;
    if (!(Number(input.max) > min)) input.max = String(min + 100);
    const max = Number(input.max);
    return [0, 0.25, 0.5, 0.81, 1].map((played) => {
      // The range moved, and the share the player would write beside it.
      input.value = String(min + played * (max - min));
      node.style.setProperty("--played", String(played));
      // Where the native thumb sits, read back from the range's value rather
      // than from `--played`: the wave has to follow the thumb, not itself.
      const fraction = (Number(input.value) - min) / (max - min);
      const w = wave.getBoundingClientRect();
      const i = input.getBoundingClientRect();
      return {
        played,
        waveEnd: w.right,
        thumbCentre: i.left + thumb / 2 + fraction * (i.width - thumb),
        trackEnd: i.right,
      };
    });
  });
  for (const { played, waveEnd, thumbCentre, trackEnd } of measured) {
    expect(Math.abs(waveEnd - thumbCentre), `at ${played}`).toBeLessThanOrEqual(
      1,
    );
    expect(waveEnd, `at ${played}`).toBeLessThanOrEqual(trackEnd);
  }
});

/**
 * The player bar's modes. `shuffledOrder`, `advance` and `retreat` are unit
 * tested; this is the wiring — that the buttons reach them, and that repeat
 * says which of its three states is on rather than only naming the action. A
 * screen reader hearing "repeat" alone could not tell.
 *
 * No audio is served: the bar renders from the restored queue, which is enough
 * to press its controls.
 */
test("carries shuffle and the three repeat states in the player bar", async ({
  page,
}, testInfo) => {
  await page.goto("/");
  const shuffle = page.getByRole("button", { name: "Shuffle" });

  if (testInfo.project.name === "mobile") {
    // Deliberate: a narrow bar keeps the transport and drops the modes. The
    // layout that came before dropped previous and next instead and kept only
    // play, which is the worse trade on a phone.
    await expect(shuffle).toBeHidden();
    await expect(
      page.getByRole("button", { name: "Repeat: off" }),
    ).toBeHidden();
    await expect(page.getByRole("button", { name: "Next" })).toBeVisible();
    return;
  }

  await expect(shuffle).toBeVisible();
  await expect(shuffle).toHaveAttribute("aria-pressed", "false");
  await shuffle.click();
  await expect(shuffle).toHaveAttribute("aria-pressed", "true");
  await shuffle.click();
  await expect(shuffle).toHaveAttribute("aria-pressed", "false");

  // Off → whole queue → this track → off.
  const repeat = page.getByRole("button", { name: "Repeat: off" });
  await repeat.click();
  await expect(
    page.getByRole("button", { name: "Repeat: the whole queue" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Repeat: the whole queue" }).click();
  await expect(
    page.getByRole("button", { name: "Repeat: this track" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Repeat: this track" }).click();
  await expect(page.getByRole("button", { name: "Repeat: off" })).toBeVisible();

  // The cover is the way back to the album that is playing.
  await expect(
    page.getByRole("link", { name: "Open the album: Vespertine" }),
  ).toHaveAttribute("href", "/albums/album-2");
});

test("shows queue artwork beside tracks without crowding times or actions", async ({
  page,
}) => {
  const songs = [
    { ...track, artwork_hash: "queue-cover-1" },
    {
      ...track,
      id: "song-2",
      title: "A much longer title in the listening queue",
      duration_ms: 92_000,
      artwork_hash: "queue-cover-2",
    },
    {
      ...track,
      id: "song-3",
      title: "Rising",
      duration_ms: 236_000,
      artwork_hash: "queue-cover-3",
    },
  ];
  await page.route("**/api/v2/queue", async (route) => {
    await route.fulfill({
      json: {
        current: "song-1",
        position_ms: 0,
        changed_by: null,
        updated_at: 1,
        songs,
      },
    });
  });
  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
    "base64",
  );
  await page.route("**/api/v2/artwork/*", async (route) => {
    await route.fulfill({ contentType: "image/png", body: png });
  });

  await page.goto("/queue");
  const rows = page.locator(".queue-list li");
  await expect(rows).toHaveCount(3);
  await rows.last().scrollIntoViewIfNeeded();
  const covers = page.locator(".queue-list img.queue-cover");
  await expect(covers).toHaveCount(3);
  await expect
    .poll(() =>
      covers.evaluateAll((images) =>
        images.every((image) => (image as HTMLImageElement).naturalWidth > 0),
      ),
    )
    .toBe(true);
  const timeEdges = await page
    .locator(".queue-duration")
    .evaluateAll((times) =>
      times.map((time) => Math.round(time.getBoundingClientRect().right)),
    );
  expect(new Set(timeEdges).size).toBe(1);

  await page.setViewportSize({ width: 320, height: 700 });
  const narrow = await rows.first().evaluate((row) => {
    const [song, time, remove] = [...row.children].map((child) =>
      child.getBoundingClientRect(),
    );
    return {
      songRight: song?.right ?? 0,
      timeLeft: time?.left ?? 0,
      timeRight: time?.right ?? 0,
      removeLeft: remove?.left ?? 0,
      overflow: document.documentElement.scrollWidth - window.innerWidth,
    };
  });
  expect(narrow.songRight).toBeLessThanOrEqual(narrow.timeLeft);
  expect(narrow.timeRight).toBeLessThanOrEqual(narrow.removeLeft);
  expect(narrow.overflow).toBeLessThanOrEqual(1);
});

/** Saved positions use their song artwork and a shared time column. */
test("shows saved tracks with artwork and one aligned time column", async ({
  page,
}) => {
  mock.bookmarks = ["Soleil Bleu", "A much longer song title", "Rising"].map(
    (title, index) => ({
      position_ms: [224_000, 236_000, 92_000][index] as number,
      comment: null,
      created_at: 1,
      updated_at: 1,
      song: {
        id: `saved-${index}`,
        title,
        artist: "Rue Delacour",
        artwork_hash: `saved-cover-${index}`,
      },
    }),
  );
  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
    "base64",
  );
  await page.route("**/api/v2/artwork/*", async (route) => {
    await route.fulfill({ contentType: "image/png", body: png });
  });

  await page.goto("/playing");
  const rows = page.locator(".bookmark-list li");
  await expect(rows).toHaveCount(3);
  await rows.last().scrollIntoViewIfNeeded();
  const covers = page.locator(".bookmark-list img.bookmark-cover");
  await expect(covers).toHaveCount(3);
  await expect
    .poll(() =>
      covers.evaluateAll((images) =>
        images.every((image) => (image as HTMLImageElement).naturalWidth > 0),
      ),
    )
    .toBe(true);
  const timeEdges = await page
    .locator(".bookmark-position")
    .evaluateAll((times) =>
      times.map((time) => Math.round(time.getBoundingClientRect().right)),
    );
  expect(new Set(timeEdges).size).toBe(1);
  await expect(page.locator(".playing-bookmarks")).toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );
  await page.setViewportSize({ width: 320, height: 700 });
  const narrow = await rows.first().evaluate((row) => {
    const [track, time, forget] = [...row.children].map((child) =>
      child.getBoundingClientRect(),
    );
    return {
      trackRight: track?.right ?? 0,
      timeLeft: time?.left ?? 0,
      timeRight: time?.right ?? 0,
      forgetLeft: forget?.left ?? 0,
      overflow: document.documentElement.scrollWidth - window.innerWidth,
    };
  });
  expect(narrow.trackRight).toBeLessThanOrEqual(narrow.timeLeft);
  expect(narrow.timeRight).toBeLessThanOrEqual(narrow.forgetLeft);
  expect(narrow.overflow).toBeLessThanOrEqual(1);
});

/**
 * Starring a track on the shuffle page does not redraw the shuffle.
 *
 * A mutation invalidates everything it could have made wrong, because a song
 * appears in albums, genres, search, history, favourites, playlists and the
 * queue. The draw is the exception: it is the one question deliberately kept
 * out of the cache, so invalidating it asks the server for a *different*
 * sample — and the list goes out from under the hand that starred it, taking
 * the starred track with it.
 */
test("keeps the shuffle still when a track in it is favourited", async ({
  page,
}) => {
  let drawn = 0;
  await page.route("**/api/v2/songs/random*", async (route) => {
    drawn += 1;
    await route.fulfill({ json: [track] });
  });
  await page.route("**/api/v2/favorites/track/*", async (route) => {
    await route.fulfill({ status: 204, body: "" });
  });

  await page.goto("/random");
  await expect(page.getByText("Army of Me")).toBeVisible();
  expect(drawn).toBe(1);

  await page.getByRole("button", { name: "Add favourite: Army of Me" }).click();
  // The button renames itself once the star is on, so the state is read from
  // the label rather than from a locator that no longer matches.
  await expect(
    page.getByRole("button", { name: "Remove favourite: Army of Me" }),
  ).toBeVisible();

  // The same track, still there, and the server was not asked to draw again.
  await expect(page.getByText("Army of Me")).toBeVisible();
  expect(drawn).toBe(1);
});

/**
 * The playing screen says what the file is.
 *
 * The facts, not the meter: the meter is five lit bars and `aria-hidden`,
 * because the band it summarises is printed in words right beside it and a
 * reader should hear that once rather than twice.
 *
 * Asserted on the rendered line rather than on the mock that fed it. The mock
 * carries these fields because `Song` declares them, so it agrees with the
 * declaration by construction — exactly the loop the lyrics test in
 * `lyrics.spec.ts` ("shows the words a track travels with") was written
 * against, where a `snake_case` envelope the server never sent was confirmed
 * by a mock built from it. What the server really answers is pinned in
 * `tests/native_api.rs`, against the bytes of a WAV.
 */
test("says what the file is on the playing screen", async ({ page }) => {
  await page.goto("/playing");

  const facts = page.locator(".file-facts");
  await expect(facts).toBeVisible();
  // Read as one string: the order is part of what makes it legible at a
  // glance, and six separate assertions would pass on a line that listed
  // them backwards.
  await expect(facts).toHaveText(
    "Lossless · FLAC · 957 kbps · 44.1 kHz · 16 bit · 2 ch · 28 MB",
  );

  // Four of five steps lit: lossless, and not the hi-res band above it.
  await expect(page.locator(".quality-meter span.on")).toHaveCount(4);
  // And hidden from assistive technology, so the band is announced once.
  await expect(page.locator(".quality-meter")).toHaveAttribute(
    "aria-hidden",
    "true",
  );
});

/**
 * RFC-022 decision 1: ReplayGain is off until asked for, and asking for it is
 * what builds the Web Audio graph — on the next play, never on page load.
 *
 * The browser's own `AudioContext` and `AudioParam` are wrapped to count
 * contexts and record the gains set, because the level a track is played at
 * cannot be heard by a test. What can be read is that no context exists while
 * the setting is off, that exactly one exists once a play follows turning it
 * on, and that the gain set is the one the tags and the preamp call for.
 */
test("levels a track by its ReplayGain only once asked to, and remembers how", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const record = window as unknown as {
      __contexts: number;
      __gains: number[];
    };
    record.__contexts = 0;
    record.__gains = [];
    const setValueAtTime = AudioParam.prototype.setValueAtTime;
    AudioParam.prototype.setValueAtTime = function (value, time) {
      record.__gains.push(value);
      return setValueAtTime.call(this, value, time);
    };
    const Original = window.AudioContext;
    window.AudioContext = class extends Original {
      constructor(options?: AudioContextOptions) {
        super(options);
        record.__contexts += 1;
      }
    };
  });
  const contexts = () =>
    page.evaluate(
      () => (window as unknown as { __contexts: number }).__contexts,
    );
  const gains = () =>
    page.evaluate(() => (window as unknown as { __gains: number[] }).__gains);

  // Off by default: playing builds nothing.
  await page.goto("/albums/album-2");
  await page.getByRole("button", { name: "Play album" }).click();
  await expect(page.locator(".player-track strong")).toHaveText("Hidden Place");
  expect(await contexts()).toBe(0);

  await page.goto("/settings/playback");
  const playback = page.locator(".settings-panel", {
    has: page.getByRole("heading", { name: "Volume levelling" }),
  });
  // The preamp changes nothing while ReplayGain is off, so it is not offered.
  await expect(playback.getByRole("slider", { name: "Preamp" })).toHaveCount(0);
  await playback.getByLabel("ReplayGain").selectOption("album");
  const preamp = playback.getByRole("slider", { name: "Preamp" });
  await expect(preamp).toBeVisible();
  await preamp.fill("3");
  await expect(playback.locator("output")).toHaveText("+3 dB");
  await expect
    .poll(() =>
      page.evaluate(() => localStorage.getItem("waveflow.replayGain")),
    )
    .toBe(JSON.stringify({ mode: "album", preampDb: 3 }));
  await expectNoWcagViolations(page);
  // Choosing it is not playing: still no graph.
  expect(await contexts()).toBe(0);

  // Remembered across a reload, and applied on the next play: the album gain
  // of −8 dB plus the 3 dB preamp, well under what the 0.95 peak allows.
  await page.goto("/albums/album-2");
  await page.getByRole("button", { name: "Play album" }).click();
  await expect(page.locator(".player-track strong")).toHaveText("Hidden Place");
  await expect.poll(contexts).toBe(1);
  await expect
    .poll(async () => (await gains()).at(-1))
    .toBeCloseTo(10 ** (-5 / 20), 4);

  // Track mode levels by the track's own gain: −7 dB for the first track.
  await page.goto("/settings/playback");
  await page.getByLabel("ReplayGain").selectOption("track");
  await page.goto("/albums/album-2");
  await page.getByRole("button", { name: "Play album" }).click();
  await expect
    .poll(async () => (await gains()).at(-1))
    .toBeCloseTo(10 ** (-4 / 20), 4);
});

/**
 * The two RFC-022 screens in the light appearance, whose contrast the dark
 * default's sweep says nothing about: the playback settings with the preamp
 * showing, and an artist page with every kind of section.
 */
test("keeps the playback settings and the discography readable in light", async ({
  page,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem("waveflow.theme", "default");
    localStorage.setItem(
      "waveflow.replayGain",
      JSON.stringify({ mode: "track", preampDb: -2 }),
    );
  });
  await page.goto("/settings/playback");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "default");
  await expect(page.getByRole("slider", { name: "Preamp" })).toHaveValue("-2");
  await expect(page.locator(".replay-gain-preamp output")).toHaveText("−2 dB");
  await expectNoWcagViolations(page);

  mock.artistExtra = {
    albums: [
      { ...albums[0], release_group: "album" },
      {
        ...albums[1],
        id: "album-4",
        title: "Single Cut",
        release_group: "single",
      },
      {
        ...albums[1],
        id: "album-5",
        title: "Best Of",
        release_group: "compilation",
      },
    ],
    appears_on: [
      {
        ...albums[0],
        id: "album-9",
        title: "Guest Spot",
        artist: "Someone Else",
      },
    ],
  };
  await page.goto("/artists/artist-1");
  await expect(page.locator(".artist-discography h3")).toHaveText([
    "Albums",
    "Singles",
    "Compilations",
    "Appears on",
  ]);
  await expectNoWcagViolations(page);
});

/**
 * A context the browser will not let run never claims the element.
 *
 * Once `createMediaElementSource` has the element, its sound reaches the
 * speakers only through the context, so a context left suspended would play
 * the music as silence while the player said it was playing. Here the
 * browser's context is replaced by one that stays suspended and whose resume
 * never settles — what a browser does when it does not count the gesture —
 * and the element must stay unclaimed, playing as it always did.
 */
test("leaves the element alone when the audio context will not run", async ({
  page,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem(
      "waveflow.replayGain",
      JSON.stringify({ mode: "album", preampDb: 0 }),
    );
    const record = window as unknown as {
      __contexts: number;
      __claims: number;
    };
    record.__contexts = 0;
    record.__claims = 0;
    const Original = window.AudioContext;
    window.AudioContext = class extends Original {
      constructor(options?: AudioContextOptions) {
        super(options);
        record.__contexts += 1;
      }
      override get state(): AudioContextState {
        return "suspended";
      }
      override resume(): Promise<void> {
        return new Promise(() => undefined);
      }
      override createMediaElementSource(element: HTMLMediaElement) {
        record.__claims += 1;
        return super.createMediaElementSource(element);
      }
    };
  });
  await page.goto("/albums/album-2");
  await page.getByRole("button", { name: "Play album" }).click();
  await expect(page.locator(".player-track strong")).toHaveText("Hidden Place");
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { __contexts: number }).__contexts,
      ),
    )
    .toBe(1);
  // A second gesture asks the same candidate again rather than making another.
  await page.locator(".album-songs .song-title").nth(1).click();
  await expect(page.locator(".player-track strong")).toHaveText("Cocoon");
  const seen = await page.evaluate(() => {
    const record = window as unknown as {
      __contexts: number;
      __claims: number;
    };
    return { contexts: record.__contexts, claims: record.__claims };
  });
  expect(seen).toEqual({ contexts: 1, claims: 0 });
});

/**
 * Signing out closes every AudioContext the player opened.
 *
 * Signing out unmounts the player, and an open context holds the audio device
 * until it is closed; a browser caps how many one page may have, so each
 * sign-in would otherwise leave one more behind. Both kinds are covered: the
 * graph a play built, and a candidate still waiting to run — the one a
 * browser that does not count the gesture leaves pending for ever.
 */
for (const kind of ["running", "pending"] as const) {
  test(`closes the ${kind} audio context when the listener signs out`, async ({
    page,
  }, testInfo) => {
    test.skip(
      testInfo.project.name !== "desktop",
      "signs out from the sidebar",
    );
    await page.addInitScript((pending) => {
      localStorage.setItem(
        "waveflow.replayGain",
        JSON.stringify({ mode: "album", preampDb: 0 }),
      );
      const contexts: AudioContext[] = [];
      (window as unknown as { __contexts: AudioContext[] }).__contexts =
        contexts;
      const Original = window.AudioContext;
      window.AudioContext = pending
        ? class extends Original {
            constructor(options?: AudioContextOptions) {
              super(options);
              contexts.push(this);
            }
            // Stays suspended until closed, and never resumes.
            override get state(): AudioContextState {
              return super.state === "closed" ? "closed" : "suspended";
            }
            override resume(): Promise<void> {
              return new Promise(() => undefined);
            }
          }
        : class extends Original {
            constructor(options?: AudioContextOptions) {
              super(options);
              contexts.push(this);
            }
          };
    }, kind === "pending");
    const states = () =>
      page.evaluate(() =>
        (window as unknown as { __contexts: AudioContext[] }).__contexts.map(
          (context) => context.state,
        ),
      );

    await page.goto("/albums/album-2");
    await page.getByRole("button", { name: "Play album" }).click();
    await expect(page.locator(".player-track strong")).toHaveText(
      "Hidden Place",
    );
    await expect
      .poll(states)
      .toEqual([kind === "running" ? "running" : "suspended"]);

    // Dispatched rather than clicked, as in the cache test: the sidebar's
    // footer sits under the fixed player bar at this viewport.
    await page.getByRole("button", { name: "Sign out" }).dispatchEvent("click");
    await expect(page).toHaveURL(/\/login$/);
    await expect.poll(states).toEqual(["closed"]);
  });
}

/**
 * Signing out stops the music, whatever the ReplayGain mode.
 *
 * The cleanup that closes the audio contexts above also paused the element,
 * but only on its way out of a graph: with ReplayGain off — the default, and
 * so the common case — nothing paused it, and the music carried on over the
 * sign-in page. The element is never in the document, so every one that was
 * asked to play is recorded, and a real file is served so that it plays.
 */
test("stops the music when the listener signs out with ReplayGain off", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "signs out from the sidebar");
  await page.addInitScript(() => {
    const played = new Set<HTMLMediaElement>();
    (window as unknown as { __played: Set<HTMLMediaElement> }).__played =
      played;
    const play = HTMLMediaElement.prototype.play;
    HTMLMediaElement.prototype.play = function () {
      played.add(this);
      return play.call(this);
    };
    const record = window as unknown as { __contexts: number };
    record.__contexts = 0;
    const Original = window.AudioContext;
    window.AudioContext = class extends Original {
      constructor(options?: AudioContextOptions) {
        super(options);
        record.__contexts += 1;
      }
    };
  });
  const audio = silentWav(60);
  await page.route("**/api/v2/tracks/*/stream-ticket", (route) =>
    route.fulfill({
      json: {
        url: "/api/v2/stream/ticket-e2e",
        expires_at: Date.now() + 60_000,
      },
    }),
  );
  await page.route("**/api/v2/stream/*", (route) =>
    route.fulfill({ body: audio, headers: { "content-type": "audio/wav" } }),
  );
  const playing = () =>
    page.evaluate(
      () =>
        [
          ...(window as unknown as { __played: Set<HTMLMediaElement> })
            .__played,
        ].filter((element) => !element.paused).length,
    );

  await page.goto("/albums/album-2");
  await page.getByRole("button", { name: "Play album" }).click();
  await expect(page.locator(".player-track strong")).toHaveText("Hidden Place");
  // Playing for real, and off the plain path: no graph was asked for.
  await expect
    .poll(() =>
      page.evaluate(() =>
        [
          ...(window as unknown as { __played: Set<HTMLMediaElement> })
            .__played,
        ].some((element) => !element.paused && element.currentTime > 0),
      ),
    )
    .toBe(true);
  expect(
    await page.evaluate(
      () => (window as unknown as { __contexts: number }).__contexts,
    ),
  ).toBe(0);

  // Dispatched rather than clicked, as in the cache test: the sidebar's
  // footer sits under the fixed player bar at this viewport.
  await page.getByRole("button", { name: "Sign out" }).dispatchEvent("click");
  await expect(page).toHaveURL(/\/login$/);
  await expect.poll(playing).toBe(0);
});

/**
 * Signing out lets go of the next track too.
 *
 * The player loads the track after the current one into a second element so
 * that skipping starts at once. That element never plays, but it holds its
 * source and may still be fetching it, for an account that has just left. Every
 * element that was asked to load is recorded, and none may keep a source once
 * the listener is signed out.
 */
test("releases the next track's preload when the listener signs out", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "signs out from the sidebar");
  await page.addInitScript(() => {
    const loaded = new Set<HTMLMediaElement>();
    (window as unknown as { __loaded: Set<HTMLMediaElement> }).__loaded =
      loaded;
    const load = HTMLMediaElement.prototype.load;
    HTMLMediaElement.prototype.load = function () {
      loaded.add(this);
      return load.call(this);
    };
  });
  const audio = silentWav(60);
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
  const sources = () =>
    page.evaluate(() =>
      [...(window as unknown as { __loaded: Set<HTMLMediaElement> }).__loaded]
        .map((element) => element.getAttribute("src"))
        .filter((source) => source !== null),
    );

  await page.goto("/albums/album-2");
  await page.getByRole("button", { name: "Play album" }).click();
  await expect(page.locator(".player-track strong")).toHaveText("Hidden Place");
  // The next track, Cocoon, is being preloaded beside the one playing. Named
  // rather than counted: the playing element is asked to load too, and its
  // own source alone would let the sign-out run before the preload exists.
  await expect.poll(sources).toContain("/api/v2/stream/ticket-song-2");

  // Dispatched rather than clicked, as in the cache test: the sidebar's
  // footer sits under the fixed player bar at this viewport.
  await page.getByRole("button", { name: "Sign out" }).dispatchEvent("click");
  await expect(page).toHaveURL(/\/login$/);
  await expect.poll(sources).toEqual([]);
});
