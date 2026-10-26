// The root route, the addresses open without a session, the session gate
// everything else sits under, and the authorisation a native client asks for.
import { createRootRoute, createRoute, redirect } from "@tanstack/react-router";

import { ensureSession } from "../../api";
import { AuthorizePage } from "../../pages/authorize-page";
import { LoginPage } from "../../pages/login-page";
import { NotFoundPage } from "../../pages/not-found-page";
import { OfflinePage } from "../../pages/offline-page";
import { SharePage } from "../../pages/share-page";
import { RootFrame, Shell } from "../shell";

export const rootRoute = createRootRoute({
  component: RootFrame,
  notFoundComponent: NotFoundPage,
});

export const loginRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/login",
  component: LoginPage,
});

/**
 * What a share link opens (RFC-020 decision 3). Beside the sign-in page and
 * not under `authed`: the person a link is sent to has no account, and a
 * session check would send them to a sign-in they cannot complete. The server
 * hands this shell out only at `/share/{token}`, to a browser asking for HTML.
 */
export const shareRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/share/$token",
  component: function ShareRoute() {
    const { token } = shareRoute.useParams();
    return <SharePage token={token} />;
  },
});

/**
 * The tracks kept for offline listening (RFC-022). Beside the sign-in page and
 * not under `authed`: it must open with no network, and a session check with
 * no server to answer it would send the listener to a sign-in that cannot
 * complete either.
 */
export const offlineRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/offline",
  component: OfflinePage,
});

/** Everything below requires a session; unauthenticated visitors land on /login. */
export const authedRoute = createRoute({
  getParentRoute: () => rootRoute,
  id: "authed",
  beforeLoad: async () => {
    if (!(await ensureSession())) {
      // Remember where the user was headed so a desktop authorisation link
      // survives the detour through sign-in instead of dropping its PKCE
      // parameters and forcing the client to start over.
      // Stored and re-read as a path only; safeInternalPath re-checks it on the
      // way out, since location.pathname can itself begin with "//".
      sessionStorage.setItem(
        "waveflow.after-login",
        window.location.pathname + window.location.search,
      );
      throw redirect({ to: "/login" });
    }
  },
  component: Shell,
});

export const authorizeRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/authorize",
  component: AuthorizePage,
});
