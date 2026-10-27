// Settings and its sections, and the addresses they had before it.
import { createRoute, redirect } from "@tanstack/react-router";

import { currentUser } from "../../api";
import { AdminPage } from "../../pages/settings/admin-page";
import { ScrobblingPage } from "../../pages/settings/scrobbling-page";
import { SettingsHub } from "../../pages/settings/settings-hub";
import {
  PlaybackSettingsPage,
  SettingsPage,
} from "../../pages/settings/settings-page";
import { SharesPage } from "../../pages/settings/shares-page";
import { UploadPage } from "../../pages/settings/upload-page";
import { authedRoute } from "./root";

/**
 * Settings, and everything about the account behind its one rail entry.
 *
 * A frame rather than a page: the section menu over whichever section is
 * open. Each section is its own route, so it can be linked to and Back walks
 * between them; `/settings` itself is the first section, Account.
 */
export const settingsRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/settings",
  component: SettingsHub,
});

export const accountSettingsRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/",
  component: SettingsPage,
});

export const playbackSettingsRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/playback",
  component: PlaybackSettingsPage,
});

// The path is the server's, not a preference. `lastfm_callback` redirects
// here when a journey completes, so spelling it any other way puts the last
// step of that journey on the not-found page.
export const scrobblingRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/scrobbling",
  component: ScrobblingPage,
});

export const sharesRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/shares",
  component: SharesPage,
});

export const uploadRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/upload",
  // A filed folder to put what is received into (RFC-012 decision 7).
  validateSearch: (search: Record<string, unknown>): { folder?: string } => ({
    folder: typeof search.folder === "string" ? search.folder : undefined,
  }),
  component: function UploadRoute() {
    const { folder } = uploadRoute.useSearch();
    return <UploadPage folder={folder} />;
  },
});

export const adminRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/admin",
  beforeLoad: () => {
    if (currentUser()?.role !== "admin") throw redirect({ to: "/" });
  },
  component: AdminPage,
});

/**
 * The addresses these sections had before they moved under Settings, kept so
 * a bookmark or a link written elsewhere still arrives.
 *
 * Redirects rather than second homes: one address per screen, and the rail's
 * Settings entry is current on every one of them because they all sit under
 * `/settings`. The router replaces the history entry it redirects from, so
 * Back from the section returns to wherever the visitor was before rather
 * than to an address that would only send them forward again.
 */
export const legacySharesRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/shares",
  beforeLoad: () => {
    throw redirect({ to: "/settings/shares", replace: true });
  },
});

export const legacyUploadRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/upload",
  // Carried across, so `/upload?folder=…` still fills that folder.
  validateSearch: (search: Record<string, unknown>): { folder?: string } => ({
    folder: typeof search.folder === "string" ? search.folder : undefined,
  }),
  beforeLoad: ({ search }) => {
    throw redirect({ to: "/settings/upload", search, replace: true });
  },
});

export const legacyAdminRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/admin",
  beforeLoad: () => {
    throw redirect({ to: "/settings/admin", replace: true });
  },
});
