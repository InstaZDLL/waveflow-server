import { describe, expect, it } from "vitest";

import type { SongCredits, TrackOverrides, TrackOverrideValues } from "./api";
import {
  buildCorrectionPatch,
  type CorrectionField,
  draftFrom,
  type FieldDraft,
  parseNames,
} from "./track-corrections";

const source = {
  title: "Army of Me",
  sort_title: null,
  year: 1995,
  track_number: 1,
  disc_number: 1,
  musicbrainz_recording_id: null,
  comment: null,
};

const uncorrected: TrackOverrideValues = {
  title: null,
  sort_title: null,
  year: null,
  track_number: null,
  disc_number: null,
  musicbrainz_recording_id: null,
  comment: null,
  artists: null,
  genres: null,
};

const song: SongCredits = {
  id: "song-1",
  library_id: "library-1",
  album_id: "album-1",
  title: "Army of Me",
  album: "Post",
  artist: "Björk",
  artist_id: "artist-1",
  artwork_hash: null,
  duration_ms: 234_000,
  track: 1,
  disc: 1,
  starred_at: null,
  user_rating: null,
  artists: [{ id: "artist-1", name: "Björk" }],
  genres: ["Trip Hop"],
};

function tracked(overrides: Partial<TrackOverrideValues> = {}): TrackOverrides {
  return { source, overrides: { ...uncorrected, ...overrides } };
}

/** The form as the editor would have it after these changes, then its body. */
function save(
  track: TrackOverrides,
  changes: Partial<Record<CorrectionField, Partial<FieldDraft>>> = {},
) {
  const draft = draftFrom(track, song);
  for (const [field, change] of Object.entries(changes)) {
    Object.assign(draft[field as CorrectionField], change);
  }
  return buildCorrectionPatch(track, song, draft);
}

describe("draftFrom", () => {
  it("starts every field from the value the track answers with", () => {
    const draft = draftFrom(
      tracked({ title: "Army of Me (Live)", artists: ["Skunk Anansie"] }),
      song,
    );
    expect(draft.title.value).toBe("Army of Me (Live)");
    expect(draft.year.value).toBe("1995");
    expect(draft.sort_title.value).toBe("");
    expect(draft.artists.value).toBe("Skunk Anansie");
    expect(draft.genres.value).toBe("Trip Hop");
  });
});

describe("buildCorrectionPatch", () => {
  it("sends nothing for a form nobody touched", () => {
    expect(save(tracked({ comment: "Remastered" }))).toEqual({
      ok: true,
      patch: {},
    });
  });

  /**
   * #177, from the client's side. The server now leaves alone what a patch does
   * not mention; that only protects a correction if the client does not mention
   * it either.
   */
  it("leaves out a correction somebody else made", () => {
    const result = save(tracked({ comment: "Remastered" }), {
      title: { value: "Army of Me (Live)" },
    });
    expect(result).toEqual({
      ok: true,
      patch: { title: "Army of Me (Live)" },
    });
  });

  it("sets a value, typed as the server expects it", () => {
    expect(save(tracked(), { year: { value: " 1996 " } })).toEqual({
      ok: true,
      patch: { year: 1996 },
    });
  });

  it("removes a correction when its field is emptied", () => {
    expect(
      save(tracked({ comment: "Remastered" }), { comment: { value: "  " } }),
    ).toEqual({ ok: true, patch: { comment: null } });
  });

  it("sends nothing when a field with no correction is emptied", () => {
    expect(save(tracked(), { year: { value: "" } })).toEqual({
      ok: true,
      patch: {},
    });
  });

  /**
   * Storing the file's own value as a correction would look like no change and
   * be one: the track would stop following its file the day the file is
   * retagged.
   */
  it("removes a correction typed back to what the file says, rather than pinning it", () => {
    expect(
      save(tracked({ title: "Army Of Me" }), {
        title: { value: "Army of Me" },
      }),
    ).toEqual({ ok: true, patch: { title: null } });
  });

  it("restores a scalar only when there is a correction to remove", () => {
    expect(save(tracked({ year: 2001 }), { year: { restore: true } })).toEqual({
      ok: true,
      patch: { year: null },
    });
    expect(save(tracked(), { year: { restore: true } })).toEqual({
      ok: true,
      patch: {},
    });
  });

  it("refuses what the server would refuse, before asking it", () => {
    expect(
      save(tracked(), {
        year: { value: "0" },
        track_number: { value: "-1" },
        disc_number: { value: "two" },
        title: { value: "Still Fine" },
      }),
    ).toEqual({ ok: false, invalid: ["year", "track_number", "disc_number"] });
  });

  it("leaves an unchanged list out", () => {
    expect(save(tracked(), { artists: { value: "  Björk \n\n" } })).toEqual({
      ok: true,
      patch: {},
    });
  });

  it("sends a changed list as a list, one name per line", () => {
    expect(save(tracked(), { artists: { value: "AC;DC\nBjörk" } })).toEqual({
      ok: true,
      patch: { artists: ["AC;DC", "Björk"] },
    });
  });

  it("reads an emptied list as crediting nobody", () => {
    expect(save(tracked(), { genres: { value: "" } })).toEqual({
      ok: true,
      patch: { genres: [] },
    });
  });

  it("restores a list only when there is a correction to remove", () => {
    expect(
      save(tracked({ artists: ["Solo"] }), { artists: { restore: true } }),
    ).toEqual({ ok: true, patch: { artists: null } });
    expect(save(tracked(), { artists: { restore: true } })).toEqual({
      ok: true,
      patch: {},
    });
  });
});

describe("parseNames", () => {
  it("splits on lines and never on the tagger's separator", () => {
    expect(parseNames(" AC;DC \r\n\n Bach/Gounod ")).toEqual([
      "AC;DC",
      "Bach/Gounod",
    ]);
  });
});
