/**
 * The setup token a server in laptop mode hands over when it opens the
 * browser on its first start (`src/app/laptop/browser.rs`):
 * `/login#setup-token=<token>`. A fragment is never sent to the server, so
 * the token is in no request line and no log; this moves it into the sign-in
 * form and out of the address bar, where it would otherwise sit in a
 * bookmark, a screenshot or a link pasted to someone.
 */
export const SETUP_TOKEN_FRAGMENT_KEY = "setup-token";

/**
 * Read the token from the fragment and remove it from the address, keeping
 * the path, the query, any other fragment parameter and the router's history
 * state. `null` when the fragment names none, and then the address is left
 * untouched.
 */
export function takeSetupTokenFromFragment(
  location: Pick<Location, "hash" | "pathname" | "search"> = window.location,
  history: Pick<History, "replaceState" | "state"> = window.history,
): string | null {
  const fragment = new URLSearchParams(location.hash.replace(/^#/, ""));
  if (!fragment.has(SETUP_TOKEN_FRAGMENT_KEY)) return null;
  const token = fragment.get(SETUP_TOKEN_FRAGMENT_KEY)?.trim() ?? "";
  fragment.delete(SETUP_TOKEN_FRAGMENT_KEY);
  const rest = fragment.toString();
  history.replaceState(
    history.state,
    "",
    `${location.pathname}${location.search}${rest ? `#${rest}` : ""}`,
  );
  return token || null;
}
