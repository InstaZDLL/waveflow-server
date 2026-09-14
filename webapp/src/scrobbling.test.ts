import { describe, expect, it } from "vitest";

import type { Play, ScrobbleDestination, ScrobbleLink } from "./api";
import {
  NAMED_UNCERTAIN,
  playsByInstant,
  returnedFrom,
  scrobbleRows,
  tracksToName,
} from "./scrobbling";

function offered(
  provider: ScrobbleDestination["provider"],
  destination: string,
  available = true,
  unavailable?: string,
): ScrobbleDestination {
  return { provider, destination, available, unavailable };
}

function linked(
  provider: ScrobbleLink["provider"],
  destination: string,
  health: ScrobbleLink["health"] = "healthy",
): ScrobbleLink {
  return {
    provider,
    destination,
    health,
    pending: 0,
    retrying: 0,
    uncertain: 0,
    oldest_pending_at: null,
    last_success_at: null,
    last_failure: null,
  };
}

describe("scrobbleRows", () => {
  it("puts each link on its own instance, not on its recipient", () => {
    const rows = scrobbleRows(
      [offered("maloja", "alice"), offered("maloja", "bob")],
      [linked("maloja", "bob")],
    );
    expect(rows.map((row) => [row.destination, row.link !== null])).toEqual([
      ["alice", false],
      ["bob", true],
    ]);
  });

  it("keeps a link whose instance the server no longer offers", () => {
    // The operator removed `bob` from the configuration. The server breaks
    // such a link rather than erasing it — the queue behind it is somebody's —
    // and a screen built from the offered instances alone would drop the row,
    // taking with it the only place to read why the listens stopped and the
    // only way to withdraw the authorisation.
    const rows = scrobbleRows(
      [offered("maloja", "alice")],
      [linked("maloja", "alice"), linked("maloja", "bob", "broken")],
    );
    expect(rows.map((row) => [row.destination, row.available])).toEqual([
      ["alice", true],
      ["bob", null],
    ]);
    expect(rows[1].link?.health).toBe("broken");
  });

  it("carries the reason an offered instance cannot be linked", () => {
    const rows = scrobbleRows(
      [offered("lastfm", "default", false, "needs an https public URL")],
      [],
    );
    expect(rows[0].unavailable).toBe("needs an https public URL");
    expect(rows[0].link).toBeNull();
  });

  it("orders by recipient, then by instance", () => {
    const rows = scrobbleRows(
      [
        offered("lastfm", "default"),
        offered("maloja", "zoe"),
        offered("maloja", "alice"),
        offered("listenbrainz", "default"),
      ],
      [],
    );
    expect(rows.map((row) => `${row.provider}/${row.destination}`)).toEqual([
      "listenbrainz/default",
      "maloja/alice",
      "maloja/zoe",
      "lastfm/default",
    ]);
  });
});

describe("playsByInstant", () => {
  it("names the track played at an instant", () => {
    const plays: Play[] = [
      { track_id: "t1", submission: true, played_at: 10 },
      { track_id: "t2", submission: true, played_at: 20 },
    ];
    expect(playsByInstant(plays).get(10)).toBe("t1");
  });

  it("names nothing when two plays share an instant", () => {
    // Guessing would tell somebody they are deciding about one track, with one
    // chance in two, on the one question the server refuses to answer for
    // them. The entry shows its time alone instead.
    const plays: Play[] = [
      { track_id: "t1", submission: true, played_at: 10 },
      { track_id: "t2", submission: true, played_at: 10 },
    ];
    expect(playsByInstant(plays).get(10)).toBeNull();
  });

  it("does not answer for an instant it never saw", () => {
    expect(playsByInstant([]).get(10)).toBeUndefined();
  });
});

describe("tracksToName", () => {
  const entry = (id: string, played_at: number) => ({
    id,
    provider: "maloja" as const,
    destination: "alice",
    played_at,
    attempts: 1,
    last_failure: null,
    updated_at: 0,
  });

  it("asks for each track once, however many entries name it", () => {
    const plays = playsByInstant([
      { track_id: "t1", submission: true, played_at: 10 },
      { track_id: "t1", submission: true, played_at: 11 },
    ]);
    expect(tracksToName([entry("a", 10), entry("b", 11)], plays)).toEqual([
      "t1",
    ]);
  });

  it("asks for nothing when the history cannot say", () => {
    const plays = playsByInstant([
      { track_id: "t1", submission: true, played_at: 10 },
      { track_id: "t2", submission: true, played_at: 10 },
    ]);
    expect(tracksToName([entry("a", 10), entry("b", 99)], plays)).toEqual([]);
  });
});

describe("returnedFrom", () => {
  it("reads the instance a completed Last.fm journey names", () => {
    expect(returnedFrom("?linked=lastfm&destination=default")).toBe("default");
  });

  it("says nothing about an ordinary visit", () => {
    expect(returnedFrom("")).toBeNull();
    expect(returnedFrom("?linked=maloja&destination=alice")).toBeNull();
    expect(returnedFrom("?destination=alice")).toBeNull();
  });

  it("refuses a name the server would never have produced", () => {
    // Anybody can type this URL, and the value is rendered. The server
    // validates a destination name to this alphabet before it can ever reach a
    // path, so a value outside it did not come from a journey.
    expect(returnedFrom("?linked=lastfm&destination=<img>")).toBeNull();
    expect(returnedFrom("?linked=lastfm&destination=")).toBeNull();
    expect(
      returnedFrom(`?linked=lastfm&destination=${"a".repeat(65)}`),
    ).toBeNull();
  });
});

describe("tracksToName, bounded", () => {
  const entry = (id: string, played_at: number) => ({
    id,
    provider: "maloja" as const,
    destination: "alice",
    played_at,
    attempts: 1,
    last_failure: null,
    updated_at: 0,
  });

  it("asks for no more names than the cap, however many entries there are", () => {
    // A destination answering ambiguously to everything produces one entry per
    // listen. Unbounded, opening this screen would fire one request per entry
    // at a server already in trouble.
    const many = Array.from({ length: 300 }, (_, index) =>
      entry(`e${index}`, index),
    );
    const plays = playsByInstant(
      many.map((one) => ({
        track_id: `t${one.played_at}`,
        submission: true,
        played_at: one.played_at,
      })),
    );
    expect(tracksToName(many, plays)).toHaveLength(NAMED_UNCERTAIN);
  });

  it("counts distinct tracks, not entries", () => {
    // Two instances of one recipient hold the same listen twice, which is two
    // entries and one name. A cap counting entries would stop at half the
    // tracks it could have named.
    const pairs = Array.from({ length: 2 * NAMED_UNCERTAIN }, (_, index) =>
      entry(`e${index}`, Math.floor(index / 2)),
    );
    const plays = playsByInstant(
      Array.from({ length: NAMED_UNCERTAIN }, (_, index) => ({
        track_id: `t${index}`,
        submission: true,
        played_at: index,
      })),
    );
    expect(tracksToName(pairs, plays)).toHaveLength(NAMED_UNCERTAIN);
  });
});
