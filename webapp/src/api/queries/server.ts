// The running server itself, as distinct from anything it holds.

import { queryOptions } from "@tanstack/react-query";
import { getServerIdentity } from "../index";

/**
 * The server's version and the source it offers.
 *
 * On the client's default freshness, not held for ever: the offer is of the
 * server's source, not the bundle's, and a server upgraded under an open page
 * runs the new version at once. Asked again when the page is next mounted or
 * focused, which is when anyone reads it — not on a timer, which would keep
 * an idle sign-in page asking for a link nobody is looking at.
 */
export const serverIdentityQuery = () =>
  queryOptions({
    queryKey: ["server-identity"],
    queryFn: getServerIdentity,
  });
