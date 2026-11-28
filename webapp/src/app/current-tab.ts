// Whether a rail link names the page already open (RFC-030).

import { serverUrl } from "../api/base";

/**
 * Whether `to`, a route as the router names it, is the page the browser is
 * on.
 *
 * The router knows its routes without Home Assistant's prefix, and the
 * browser's address carries it: compared as they are, inside the panel the
 * two never matched, and a click on the current tab, which cancels a switch
 * still loading, started another one instead.
 */
export function isCurrentTab(to: string): boolean {
  return window.location.pathname === serverUrl(to);
}
