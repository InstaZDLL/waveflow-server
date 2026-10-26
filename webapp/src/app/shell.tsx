// The frames every screen sits inside: the root, and the signed-in shell
// with its rail, its bars and the player.
import { Outlet, useMatches } from "@tanstack/react-router";
import { useEffect } from "react";

import { CoverLight } from "../components/cover-light";
import { Waiting } from "../components/page-shell";
import { dismissSplash } from "../lib/splash";
import { PlayerBar, PlayerProvider } from "../player/player";
import { useI18n } from "../state/i18n";
import {
  LibraryPicker,
  LibraryScopeProvider,
  useLibraryScope,
} from "../state/library-scope";
import { Brand, Navigation } from "./navigation";

/**
 * Takes the first-load screen away once the application has committed a
 * screen of its own.
 *
 * Renders nothing. An effect runs after the commit it belongs to, so by the
 * time this fires the router has put either the sign-in page or the shell in
 * the document — which is what the splash was covering.
 *
 * **Inside the router's root component**, so the commit this effect belongs
 * to is the one that put a screen in the document. As a sibling of
 * `RouterProvider` it fired on the first commit of the tree, which the
 * router need not have filled yet — the 300ms floor hid that, and a floor
 * hiding an ordering is weaker than an ordering that is right.
 *
 * Not in `loginRoute` and `authedRoute` instead, which was the other
 * suggestion: `notFoundComponent` belongs to the root and renders inside
 * neither, so an unknown address on a first visit would have kept the mark
 * until the ten-second ceiling.
 *
 * **Not on the scope being ready** either, which would hold the mark over the
 * one round trip the shell still waits on. That wait has `Waiting` already,
 * with `role="status"`, and is usually finished inside the floor. Covering
 * it with the mark would mean the screen outliving a failure it cannot
 * describe.
 */
function SplashGate() {
  useEffect(() => {
    dismissSplash();
  }, []);
  return null;
}

/**
 * Holds the catalogue back until the active library is known. One round trip,
 * and it buys the guarantee that no screen renders the whole catalogue before
 * narrowing to one library — which is what the unscoped first request did.
 */
function ScopedOutlet() {
  const { ready } = useLibraryScope();
  // Announced rather than blank: a screen reader on an empty `main` has
  // nothing to say, and the wait is a round trip the visitor did not ask for.
  return ready ? <Outlet /> : <Waiting />;
}

/** Compose the authenticated desktop and mobile application chrome. */
export function Shell() {
  const { t } = useI18n();
  // Read from the routes rather than set by each page, so the width holds
  // while a page is still loading and the hub's sections share it.
  const workspace = useMatches({
    select: (matches) =>
      matches.some((match) => match.staticData.layout === "workspace"),
  });
  return (
    <LibraryScopeProvider>
      <PlayerProvider>
        <CoverLight />
        <a className="skip-link" href="#main-content">
          {t("nav.skip")}
        </a>
        <div className="shell">
          <aside className="sidebar">
            <Brand />
            <LibraryPicker />
            {/* The rail and, under it, the account: who is signed in, the
                Settings entry and Sign out. The theme and the language used to
                sit here too; they are settings and not destinations, so they
                live on the Settings page — and only there, because a copy left
                behind would give two ways to ask one question. */}
            <Navigation />
          </aside>
          <header className="mobile-header">
            <Brand />
            <LibraryPicker />
          </header>
          <main
            id="main-content"
            tabIndex={-1}
            className={workspace ? "workspace" : undefined}
          >
            <ScopedOutlet />
          </main>
          <Navigation mobile />
          <PlayerBar />
        </div>
      </PlayerProvider>
    </LibraryScopeProvider>
  );
}

/**
 * Everything every route sits inside: the outlet, and the one effect that
 * takes the first-load screen away.
 */
export function RootFrame() {
  return (
    <>
      <SplashGate />
      <Outlet />
    </>
  );
}
