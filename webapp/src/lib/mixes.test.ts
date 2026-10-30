import { beforeAll, describe, expect, it } from "vitest";

import type { MixSummary } from "../api";
import { loadLocale, translate } from "../state/i18n";
import { barShares, listeningTime, mixTitle } from "./mixes";

const mix = (overrides: Partial<MixSummary>): MixSummary => ({
  id: "most-played",
  kind: "most_played",
  genre: null,
  decade: null,
  track_count: 3,
  artwork_hashes: [],
  ...overrides,
});

// French is a chunk of its own, fetched before anything reads it.
beforeAll(() => loadLocale("fr"));

describe("mix titles", () => {
  const en = (
    key: Parameters<typeof translate>[1],
    values?: Record<string, string | number>,
  ) => translate("en", key, values);
  const fr = (
    key: Parameters<typeof translate>[1],
    values?: Record<string, string | number>,
  ) => translate("fr", key, values);

  it("names each kind in the listener's language", () => {
    expect(mixTitle(mix({}), en)).toBe("Most played");
    expect(mixTitle(mix({ kind: "never_played" }), fr)).toBe("Jamais écoutés");
    expect(mixTitle(mix({ kind: "genre", genre: "Jazz" }), en)).toBe(
      "Jazz mix",
    );
    expect(mixTitle(mix({ kind: "decade", decade: 1990 }), en)).toBe(
      "The 1990s",
    );
    expect(mixTitle(mix({ kind: "decade", decade: 1990 }), fr)).toBe(
      "Les années 1990",
    );
  });
});

describe("a mix that cannot be opened", () => {
  it("is named in both languages", () => {
    expect(translate("en", "mixes.failed", { title: "Jazz mix" })).toBe(
      "“Jazz mix” could not be opened. Try again.",
    );
    expect(translate("fr", "mixes.failed", { title: "Mix Jazz" })).toBe(
      "Impossible d’ouvrir « Mix Jazz ». Réessayez.",
    );
  });
});

describe("listening time", () => {
  const en = (
    key: Parameters<typeof translate>[1],
    values?: Record<string, string | number>,
  ) => translate("en", key, values);

  it("reads as minutes under an hour and as hours and minutes above", () => {
    expect(listeningTime(0, en)).toBe("0 minutes");
    expect(listeningTime(60_000, en)).toBe("1 minute");
    expect(listeningTime(3_725_000, en)).toBe("1 h 2 min");
  });
});

describe("bar shares", () => {
  it("measures each bucket against the busiest, and survives an empty period", () => {
    expect(barShares([0, 2, 4])).toEqual([0, 0.5, 1]);
    expect(barShares([0, 0])).toEqual([0, 0]);
    expect(barShares([])).toEqual([]);
  });
});
