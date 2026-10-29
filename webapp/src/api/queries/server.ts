// The running server itself, as distinct from anything it holds.

import { queryOptions } from "@tanstack/react-query";
import { getServerIdentity } from "../index";

/**
 * The server's version and the source it offers.
 *
 * Never stale: the page was served by this version, and one upgraded under it
 * keeps running the old bundle, which this answer then matches, until it is
 * reloaded.
 */
export const serverIdentityQuery = () =>
  queryOptions({
    queryKey: ["server-identity"],
    queryFn: getServerIdentity,
    staleTime: Number.POSITIVE_INFINITY,
  });
