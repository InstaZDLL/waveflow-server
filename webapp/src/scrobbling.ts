import type {
  Play,
  ScrobbleDestination,
  ScrobbleLink,
  ScrobbleProvider,
  UncertainScrobble,
} from "./api";

/**
 * The pure half of the scrobbling screen — RFC-010.
 *
 * Kept apart from the component for the reason `uploads.ts` is: these are the
 * rules a test can hold still, and every one of them exists because the naive
 * reading of the API loses something a person needs.
 */

/** One instance, with the account's authorisation when it has one. */
export type ScrobbleRow = {
  provider: ScrobbleProvider;
  destination: string;
  /** `null` when the operator no longer declares this instance. */
  available: boolean | null;
  unavailable?: string;
  link: ScrobbleLink | null;
};

const PROVIDER_ORDER: ScrobbleProvider[] = ["listenbrainz", "maloja", "lastfm"];

function rank(provider: ScrobbleProvider): number {
  const index = PROVIDER_ORDER.indexOf(provider);
  return index === -1 ? PROVIDER_ORDER.length : index;
}

/**
 * Every instance the screen has to show: the ones this server declares, and
 * the ones this account is still linked to.
 *
 * The second half is the one a listing built from either side alone loses. An
 * operator who removes an instance from the configuration leaves the links to
 * it behind — the server breaks them rather than erasing them, because the
 * queue behind them is somebody's. A screen that iterated the declared
 * destinations would drop those rows silently, and with them the only place
 * the person could read why their listens stopped leaving, or withdraw the
 * authorisation. `available: null` is that row: linked, and no longer offered.
 */
export function scrobbleRows(
  destinations: ScrobbleDestination[],
  links: ScrobbleLink[],
): ScrobbleRow[] {
  const linkOf = new Map(
    links.map((link) => [`${link.provider}/${link.destination}`, link]),
  );
  const rows: ScrobbleRow[] = destinations.map((destination) => ({
    provider: destination.provider,
    destination: destination.destination,
    available: destination.available,
    unavailable: destination.unavailable,
    link:
      linkOf.get(`${destination.provider}/${destination.destination}`) ?? null,
  }));
  const declared = new Set(
    destinations.map(
      (destination) => `${destination.provider}/${destination.destination}`,
    ),
  );
  for (const link of links) {
    if (declared.has(`${link.provider}/${link.destination}`)) continue;
    rows.push({
      provider: link.provider,
      destination: link.destination,
      available: null,
      link,
    });
  }
  return rows.sort(
    (left, right) =>
      rank(left.provider) - rank(right.provider) ||
      left.destination.localeCompare(right.destination),
  );
}

/**
 * The track each ambiguous listen was, when the account's own history can say.
 *
 * An entry carries no title — decision 12 keeps the envelope out of the API —
 * and `played_at` is what it carries instead, precisely so a client can match
 * it against a listen it already holds. The two are written in one
 * transaction from one value, so the match is exact rather than approximate;
 * nothing here searches near a timestamp.
 *
 * **A timestamp naming two plays names neither.** Two tracks recorded at the
 * same millisecond would otherwise have the screen tell somebody they are
 * deciding about one of them, with one chance in two, on the one question the
 * server refuses to answer in their place. Such a timestamp resolves to
 * nothing and the entry shows its time alone.
 */
export function playsByInstant(plays: Play[]): Map<number, string | null> {
  const found = new Map<number, string | null>();
  for (const play of plays) {
    found.set(play.played_at, found.has(play.played_at) ? null : play.track_id);
  }
  return found;
}

/**
 * How many distinct tracks the ambiguous listing resolves names for.
 *
 * Each one costs a request, and they all leave at once. The list is usually
 * short — an ambiguous listen needs a connection that broke mid-submission —
 * but a destination answering ambiguously to everything produces one entry per
 * listen, which RFC-010 says in as many words. Unbounded, opening this screen
 * would then fire hundreds of requests at the server that is already having
 * trouble.
 *
 * **What is capped is the naming, never the listing.** Every entry is shown
 * whatever happens: each is a decision that is owed, and hiding one is the
 * single outcome this screen exists to prevent. Past the cap an entry shows
 * its time, its instance and its cause, which is what it would have shown
 * anyway had the history not reached back that far.
 *
 * Fifty, like the recently-played screen, for the same reason and with the
 * same shape.
 */
export const NAMED_UNCERTAIN = 50;

/** The distinct tracks an uncertain listing needs names for, at most [`NAMED_UNCERTAIN`]. */
export function tracksToName(
  entries: UncertainScrobble[],
  plays: Map<number, string | null>,
): string[] {
  const wanted = new Set<string>();
  for (const entry of entries) {
    const track = plays.get(entry.played_at);
    if (track) wanted.add(track);
    if (wanted.size === NAMED_UNCERTAIN) break;
  }
  return [...wanted];
}

/**
 * The instance a completed Last.fm journey came back naming, if this really is
 * that return.
 *
 * The callback finishes the link itself and redirects here with
 * `?linked=lastfm&destination=…`, carrying no token. Read as a claim about
 * what just happened, and nothing else: the value is shown, never sent
 * anywhere and never used to build a request, and anybody can type this URL.
 * The listing loaded beside it is what actually says whether a link exists.
 */
export function returnedFrom(search: string): string | null {
  const params = new URLSearchParams(search);
  if (params.get("linked") !== "lastfm") return null;
  const destination = params.get("destination");
  // The server validates a destination name to this alphabet before it ever
  // reaches a path; a value outside it did not come from the server.
  return destination && /^[A-Za-z0-9._-]{1,64}$/.test(destination)
    ? destination
    : null;
}
