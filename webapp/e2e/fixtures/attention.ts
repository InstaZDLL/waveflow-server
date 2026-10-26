// The credits waiting for a decision, as Needs attention is served them.

import type { Page } from "@playwright/test";

import { song } from "./catalogue";

/**
 * "Needs attention" (RFC-015), as the server answers it: the lists and their
 * counts, the joined credits with the fragments the library already knows,
 * and the decisions. Held here so a test can watch what the page writes.
 */
export async function serveAttention(page: Page) {
  const state = {
    candidates: [
      {
        artist_id: "artist-joined",
        name: "Ashreveal, Ashish Swargiary",
        track_count: 1,
        fragments: [
          { name: "Ashreveal", artist_id: "artist-known", album_count: 2 },
          { name: "Ashish Swargiary", artist_id: null, album_count: 0 },
        ],
      },
    ],
    decisions: [] as Array<Record<string, unknown>>,
    writes: [] as Array<{ method: string; path: string; body: unknown }>,
    asks: 0,
    /** Answer the next decision with this status, as if another owner had
     * decided first: the credit leaves the list on the server. */
    refuseWith: null as number | null,
  };
  const base = "**/api/v2/libraries/library-1";
  await page.route(`${base}/attention`, (route) => {
    state.asks += 1;
    return route.fulfill({
      json: [
        { category: "split_credits", count: state.candidates.length },
        { category: "missing_artist", count: 0 },
        { category: "missing_album", count: 0 },
        { category: "missing_year", count: 1 },
        { category: "missing_track_number", count: 0 },
        { category: "missing_cover", count: 0 },
        { category: "album_year_conflict", count: 0 },
        { category: "compilation_no_album_artist", count: 0 },
        { category: "duplicate_track_number", count: 0 },
        { category: "track_number_gap", count: 0 },
        { category: "probable_duplicate", count: 0 },
      ],
    });
  });
  await page.route(`${base}/attention/credits`, (route) =>
    route.fulfill({ json: state.candidates }),
  );
  await page.route(`${base}/attention/tracks?*`, (route) => {
    const category = new URL(route.request().url()).searchParams.get(
      "category",
    );
    const twice = (index: number, title: string, group: number) => ({
      ...song(index, title, 0, false),
      duplicate_group: group,
    });
    return route.fulfill({
      json:
        category === "missing_year"
          ? [song(4, "Yearless", 0, false)]
          : category === "probable_duplicate"
            ? [
                twice(5, "Echo", 1),
                twice(6, "Echo (copy)", 1),
                twice(7, "Other", 2),
                twice(8, "Other (copy)", 2),
              ]
            : [],
    });
  });
  await page.route(`${base}/credit-decisions`, async (route) => {
    const request = route.request();
    if (request.method() === "GET") {
      await route.fulfill({ json: state.decisions });
      return;
    }
    const body = request.postDataJSON();
    state.writes.push({ method: "POST", path: "credit-decisions", body });
    if (state.refuseWith !== null) {
      const status = state.refuseWith;
      state.refuseWith = null;
      state.candidates = [];
      await route.fulfill({ status, json: { error: "conflict" } });
      return;
    }
    const candidate = state.candidates.find(
      (entry) => entry.artist_id === body.artist_id,
    );
    state.candidates = state.candidates.filter(
      (entry) => entry.artist_id !== body.artist_id,
    );
    const decision = {
      id: "decision-1",
      credit: candidate?.name ?? "?",
      decision: body.decision,
      ...(body.decision === "split"
        ? { parts: candidate?.fragments.map((fragment) => fragment.name) }
        : {}),
      decided_by: "listener",
      decided_at: 1_700_000_000_000,
    };
    state.decisions = [decision];
    await route.fulfill({ status: 201, json: decision });
  });
  await page.route(`${base}/credit-decisions/*`, async (route) => {
    const request = route.request();
    state.writes.push({
      method: request.method(),
      path: new URL(request.url()).pathname.split("/").at(-1) ?? "",
      body: null,
    });
    state.decisions = [];
    await route.fulfill({ status: 204, body: "" });
  });
  return state;
}
