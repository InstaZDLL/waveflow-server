import { call } from "./client";

export const scrobble = (trackId: string, submission: boolean) =>
  call<void>("/api/v2/scrobbles", {
    method: "POST",
    body: JSON.stringify({ track_id: trackId, submission }),
  });

/**
 * External scrobbling — RFC-010.
 *
 * The operator names instances and the account picks a name: no call here
 * sends a URL, and no answer carries one. `provider` is the recipient
 * (`listenbrainz`, `maloja`, `lastfm`) and `destination` is which of its
 * instances this server declares; the pair names every link and every route.
 */
export type ScrobbleProvider = "listenbrainz" | "maloja" | "lastfm";

/**
 * The cases this build has been taught to word.
 *
 * Exhaustive over what the server sends *today*, which is what a screen's
 * translation table must cover.
 */
export type KnownScrobbleUnavailable =
  | "no_application_configured"
  | "browser_journey_needs_https";

/**
 * Why a declared instance cannot be linked — a case, not a sentence.
 *
 * The server sent its own English prose here until 2026-09-15, which this
 * client printed verbatim: a French reader was told in English what to change
 * in a configuration file. The wording belongs to whoever is doing the
 * telling, so the wire carries only the case.
 *
 * Open on purpose. A server is free to be newer than the client reading it, so
 * a type naming only today's cases would be the same untruth this file has
 * just been cleared of: an assertion about the wire that the wire never made.
 * `string & {}` keeps the known cases in autocomplete while admitting the rest,
 * so handling an unrecognised one needs no cast to express.
 */
export type ScrobbleUnavailable = KnownScrobbleUnavailable | (string & {});

/** One instance a member may link, and whether this server can link it now. */
export type ScrobbleDestination = {
  provider: ScrobbleProvider;
  destination: string;
  available: boolean;
  /**
   * Why not, when it is not. Present only for an unavailable one, which is why
   * a screen must not read it as the reason a link failed.
   */
  unavailable?: ScrobbleUnavailable;
};

/**
 * What a link reports: a state and four counters, never an echo of a listen.
 *
 * `healthy` cannot mean "the token is still good" — a valid link with three
 * thousand listens waiting since this morning is broken in every sense that
 * matters, and that is what `degraded` says. `broken` is a refused credential,
 * or a destination this server can no longer reach under the name the link was
 * made against.
 */
export type ScrobbleLink = {
  provider: ScrobbleProvider;
  destination: string;
  health: "healthy" | "degraded" | "broken";
  pending: number;
  retrying: number;
  uncertain: number;
  oldest_pending_at: number | null;
  last_success_at: number | null;
  last_failure: string | null;
};

/**
 * One listen whose fate nobody knows, named so that it can be answered.
 *
 * It carries no title and no artists: what identifies it to a person is
 * `played_at`, which matches a listen they already hold from their history,
 * and `destination`, which says which instance may already have it.
 */
export type UncertainScrobble = {
  id: string;
  provider: ScrobbleProvider;
  destination: string;
  played_at: number;
  attempts: number;
  last_failure: string | null;
  updated_at: number;
};

export const listScrobbleDestinations = () =>
  call<ScrobbleDestination[]>("/api/v2/scrobble-destinations");

export const listScrobbleLinks = () =>
  call<ScrobbleLink[]>("/api/v2/scrobble-links");

/**
 * Authorises a destination with a secret the person holds.
 *
 * `PUT` because the instance names the resource: presenting a second token
 * leaves one link, not two. Listens already queued stay attached to the
 * authorisation they were queued under.
 */
export const linkScrobble = (
  provider: ScrobbleProvider,
  destination: string,
  secret: string,
) =>
  call<void>(`/api/v2/scrobble-links/${provider}/${destination}`, {
    method: "PUT",
    body: JSON.stringify({ secret }),
  });

/** Withdraws it. Succeeds whether or not anything was linked. */
export const unlinkScrobble = (
  provider: ScrobbleProvider,
  destination: string,
) =>
  call<void>(`/api/v2/scrobble-links/${provider}/${destination}`, {
    method: "DELETE",
  });

/**
 * Opens the Last.fm journey and answers where to send the browser.
 *
 * The response also sets an `HttpOnly` cookie scoped to the return path, and
 * the return is refused without it. This client is served from the API's own
 * origin, so `fetch`'s default `same-origin` credentials carry it; one served
 * from elsewhere would have to ask for `include`.
 *
 * The journey lasts twelve minutes, is good once, and has to finish in the
 * browser that opened it. Last.fm brings it back to a route that completes the
 * link itself and redirects to `/settings/scrobbling` carrying no token — so
 * nothing here handles the return.
 */
export const authorizeLastFm = (destination: string) =>
  call<{ authorize_url: string; expires_in: number }>(
    `/api/v2/scrobble-links/lastfm/authorize/${destination}`,
    { method: "POST" },
  );

export const listUncertainScrobbles = () =>
  call<UncertainScrobble[]>("/api/v2/scrobble-queue/uncertain");

/** Throws it away. The listen is never submitted and stops being asked about. */
export const discardUncertainScrobble = (entryId: string) =>
  call<void>(`/api/v2/scrobble-queue/uncertain/${entryId}`, {
    method: "DELETE",
  });

/**
 * Sends it again, accepting that the destination may already hold it.
 *
 * Granted once per entry: a second call answers 404, and so does an entry
 * belonging to somebody else. The ambiguous entry stays in the record exactly
 * as it happened — erasing it would falsify the only trace explaining why the
 * destination may hold the listen twice.
 */
export const retryUncertainScrobble = (entryId: string) =>
  call<{ id: string }>(`/api/v2/scrobble-queue/uncertain/${entryId}/retry`, {
    method: "POST",
  });
