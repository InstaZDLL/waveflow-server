import {
  ApiError,
  artworkUrls,
  renewFor,
  session,
  sessionGeneration,
  settledArtworkUrls,
} from "./client";

/** What is already held for this artwork, or `null` if nothing is yet. */
export function cachedArtworkUrl(id: string | null): string | null {
  return id ? (settledArtworkUrls.get(id) ?? null) : null;
}

async function loadArtworkUrl(
  id: string,
  retry = true,
): Promise<string | null> {
  const headers = new Headers();
  if (session) headers.set("authorization", `Bearer ${session.access_token}`);
  // Whose session this attempt speaks for; see `renewFor`.
  const generation = sessionGeneration;
  const response = await fetch(`/api/v2/artwork/${encodeURIComponent(id)}`, {
    headers,
  });
  if (response.status === 401 && retry && (await renewFor(generation))) {
    return loadArtworkUrl(id, false);
  }
  // The same distinction `canvasUrl` makes above, and for the same reason: a
  // 404 is the server saying it holds no such artwork, which is an answer; any
  // other failure is a server that could not answer, which is not one. They
  // collapsed into the same `null` here and were then held for the session, so
  // a single blip left a cover grey until the tab was reloaded.
  //
  // An id here is a content hash — every caller passes `artwork_hash` — so a
  // 404 is about immutable content and is worth keeping. Re-asking it on every
  // mount would put back the request storm lazy loading exists to prevent.
  if (response.status === 404) return null;
  if (!response.ok) {
    throw new ApiError(response.status, `GET /api/v2/artwork/${id}`);
  }
  return URL.createObjectURL(await response.blob());
}

/** Resolve authenticated artwork to a browser-safe object URL for <img>. */
export function artworkUrl(id: string | null): Promise<string | null> {
  if (!id) return Promise.resolve(null);
  const cached = artworkUrls.get(id);
  if (cached) return cached;
  const pending: Promise<string | null> = loadArtworkUrl(id).then(
    (url) => {
      // Only while this is still the request being waited on. A session change
      // empties both maps and revokes the object URLs, and a load started for
      // the account that left would otherwise land here afterwards — putting a
      // revoked URL back under a key the next account reads.
      if (artworkUrls.get(id) === pending) settledArtworkUrls.set(id, url);
      return url;
    },
    () => {
      // Not an answer, so nothing is remembered: the entry is dropped and the
      // next mount asks again. `null` is returned rather than rethrown because
      // a cover that could not be fetched is a placeholder, not a broken page.
      if (artworkUrls.get(id) === pending) artworkUrls.delete(id);
      return null;
    },
  );
  artworkUrls.set(id, pending);
  return pending;
}
