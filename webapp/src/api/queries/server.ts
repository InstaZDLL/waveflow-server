// The running server itself, as distinct from anything it holds.

import { queryOptions } from "@tanstack/react-query";
import { getServerIdentity } from "../index";

/**
 * The server's version and the source it offers.
 *
 * On the client's default freshness, not held for ever: the offer is of the
 * server's source, not the bundle's, and a server upgraded under an open page
 * runs the new version at once. Asked again, once stale, when the page is next
 * mounted or focused, which is when anyone reads it — not on a timer, which
 * would keep an idle sign-in page asking for a link nobody is looking at.
 *
 * Focus is switched back on here alone. The client turns it off so a library
 * left in the background does not answer a return with a round of requests;
 * this is one small request, and only the sign-in page and Settings hold it.
 */
export const serverIdentityQuery = () =>
  queryOptions({
    queryKey: ["server-identity"],
    queryFn: getServerIdentity,
    refetchOnWindowFocus: true,
  });
