// The mock server's mutable state, which the tests read, replace and reset.

import {
  type Client,
  correctableTrack,
  freshDestinations,
  freshUncertain,
  freshUploadSession,
  library,
  type linkState,
  type missingTrack,
  type Offer,
  type song,
} from "./catalogue";

/**
 * Everything the mock server keeps between requests, which the tests read,
 * replace and reset. Fields of one object rather than module-level `let`s,
 * because a binding imported from another module cannot be reassigned.
 */
class MockState {
  missingTracks: Array<ReturnType<typeof missingTrack>> = [];
  /**
   * What a purge took out of sight and the server still holds, restorable,
   * as `GET /libraries/{id}/purged-tracks` answers it.
   */
  purgedTracks: Array<ReturnType<typeof missingTrack>> = [];
  /** What each purge asked for, in order. */
  purgeAsks: unknown[] = [];
  /** What each restoration asked for, in order. */
  restoreAsks: unknown[] = [];

  /** Replaced by the scoping test; one library elsewhere, so no picker appears. */
  libraries: Array<ReturnType<typeof library>> = [
    library("library-1", "Ma musique"),
  ];

  /** Flipped by the two tests that check a failure is shown as one. */
  membersFail = false;
  tokensFail = false;

  ownClients: Client[] = [];
  /**
   * The Subsonic credential this account holds, or `null` for none.
   *
   * Two states and not one: the panel says different things about "you hold
   * none" and "the request has not answered", so the mock has to be able to say
   * the first.
   */
  ownSubsonicCredential: { created_at: number; updated_at: number } | null =
    null;
  /**
   * The saved positions the playing screen lists under the track.
   *
   * Empty by default, which is what it was: `/api/v2/bookmarks` fell through to
   * the mock's 404, so the list the screen renders was never drawn by any test.
   * One that needs the page taller than the viewport fills it.
   */
  bookmarks: Array<{
    position_ms: number;
    comment: string | null;
    created_at: number;
    updated_at: number;
    song: {
      id: string;
      title: string;
      artist: string | null;
      artwork_hash?: string | null;
    };
  }> = [];
  /** Every registration the clients route received. */
  clientWrites: string[] = [];
  /** Held open by the double-press test; instant for everyone else. */
  clientGate: Promise<void> = Promise.resolve();
  /** Every method the credential route received, in order. */
  subsonicWrites: string[] = [];
  /** Held open by the double-press test; instant for everyone else. */
  subsonicGate: Promise<void> = Promise.resolve();

  scanFails = false;
  /** The stream never answers at all, which is what a lost network looks like. */
  scanDrops = false;

  /** Resolved by default; one test replaces it to stall `album-1`. */
  slowAlbum: Promise<void> = Promise.resolve();

  /**
   * What this account played, in the order the server ranked it. Reassigned by
   * the test that empties it: an artist with no history must carry no heading
   * rather than a heading over nothing.
   */
  artistPlayed: Array<ReturnType<typeof song>> = [];
  /** The limits the page asked for, so a screenful can be told from a catalogue. */
  topSongAsks: Array<string | null> = [];
  /** How many times the artist page was asked for. */
  artistAsks = 0;
  /**
   * Fields laid over the artist's answer: the RFC-022 sections and appearances.
   * Empty by default, which is what a server from before RFC-022 sends.
   */
  artistExtra: Record<string, unknown> = {};
  /**
   * The `enrichment` object of the artist's nth answer, counting from one.
   * Undefined leaves it out, as a server from before RFC-011 does.
   */
  enrichmentOf: (ask: number) => unknown = () => undefined;
  /** Every artist answer after the first is a 500. */
  artistFailsAfterFirst = false;

  /** Every import request, so a dry run can be told from a commit. */
  importAsks: Array<{ dryRun: boolean; name: string | null }> = [];
  /** What the export route was asked for, and what it answered. */
  exportAsks: string[] = [];

  /** Every write the radio page sent, so one click can be told from two. */
  radioWrites: string[] = [];
  /** Held open by the double-press test; instant for everyone else. */
  radioGate: Promise<void> = Promise.resolve();

  /**
   * The account's radio stations, which this harness really does keep: the page
   * writes and reads them back, so a mock answering a fixed list would pass a
   * delete that deleted nothing.
   */
  stations: Array<{
    id: string;
    name: string;
    stream_url: string;
    homepage_url: string | null;
    image_url: string | null;
    artwork_hash: string | null;
    created_at: number;
    updated_at: number;
  }> = [];
  /** Every image address a station write carried, as the server would fetch it. */
  stationImages: Array<string | null | undefined> = [];

  correctable = correctableTrack();

  /** Every correction body sent, in order. */
  corrections: unknown[] = [];

  /** What the upload mock received, in order. */
  uploadOffers: Offer[] = [];
  uploadChunks: Array<{ index: number; bytes: number[] }> = [];
  uploadCommits: string[] = [];
  /** The one session the mock holds, advanced by each fragment it writes. */
  uploadSession = freshUploadSession();
  /**
   * Set by the resume test: the fragment at this index is written, and its
   * acknowledgement lost — the session advances and the client is told 409.
   */
  loseAcknowledgementOf: number | null = null;

  destinations = freshDestinations();
  scrobbleLinks: Array<ReturnType<typeof linkState>> = [];
  uncertain = freshUncertain();
  /** Every link, unlink and answer the mock received, in order. */
  scrobbleWrites: Array<{ method: string; path: string; body?: unknown }> = [];
  /** Where the Last.fm journey says to send the browser. */
  lastFmAuthorizeUrl = "https://www.last.fm/api/auth/?api_key=k&cb=back";

  /**
   * The words the mock holds for each track. Absent is a track without any.
   *
   * Shaped like `src/lyrics.rs` answers, `camelCase` envelope included — written
   * from the server rather than from `LyricsList` in `webapp/src/api/tracks.ts`, which
   * is how that type spent a release claiming `structured_lyrics` while every
   * read of it threw.
   */
  lyricSheets = new Map<
    string,
    Array<{
      displayArtist: string | null;
      displayTitle: string;
      lang: string;
      synced: boolean;
      line: Array<{ start?: number; value: string }>;
      /** Where the server says the sheet came from: `lrclib` since RFC-024. */
      source?: string;
    }>
  >();

  /**
   * The mixes `GET /api/v2/mixes` answers (RFC-024). Empty by default, which is
   * what a server answers for an account with nothing to suggest — and what
   * keeps the home page every other test reads exactly as it was.
   */
  mixes: Array<{
    id: string;
    kind: string;
    genre: string | null;
    decade: number | null;
    track_count: number;
    artwork_hashes: string[];
  }> = [];
  /** Every mix id the mock was asked for, in order. */
  mixAsks: string[] = [];
  /** Set by the test that checks a mix which cannot be read says so. */
  mixFails = false;
  /** Every period the statistics route was asked for, in order. */
  statsAsks: string[] = [];

  /** Held open by the late-sheet test; instant for everyone else. */
  lyricsGate: Promise<void> = Promise.resolve();
  /** The loop the mock holds for each track, as bytes. Absent is none. */
  canvases = new Map<string, number[]>();
  /** Every placement and removal the mock received, in order. */
  canvasWrites: Array<{ method: string; track: string; bytes?: number[] }> = [];
  /** Set by a test to refuse the next placement with this status. */
  refuseCanvasWith: number | null = null;
  /**
   * Held by the playing page test, so a loop's bytes arrive only once it has
   * looked at the element: they are not a video, and the browser drops the
   * element the moment it finds that out.
   */
  canvasStreamGate: Promise<void> = Promise.resolve();
}

/** The one instance, shared by the mock and every spec in a worker. */
export const mock = new MockState();
