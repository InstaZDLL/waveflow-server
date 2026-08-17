import {
  createRootRoute,
  createRoute,
  createRouter,
  Link,
  Outlet,
  RouterProvider,
  redirect,
  useNavigate,
} from "@tanstack/react-router";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { currentUser, ensureSession, logout } from "./api";
import { I18nProvider, LanguagePicker, useI18n } from "./i18n";
import { Icon, type IconName } from "./icons";
import {
  AdminPage,
  AlbumPage,
  AlbumsPage,
  ArtistPage,
  ArtistsPage,
  AuthorizePage,
  FavoritesPage,
  LoginPage,
  NotFoundPage,
  PlaylistsPage,
  QueuePage,
  SearchPage,
  SharesPage,
} from "./pages";
import { PlayerBar, PlayerProvider } from "./player";
import { PreferencesProvider, ThemePicker } from "./preferences";
import "./styles.css";

const navigation: Array<{
  to:
    | "/"
    | "/artists"
    | "/search"
    | "/favourites"
    | "/playlists"
    | "/queue"
    | "/shares"
    | "/admin";
  label:
    | "Albums"
    | "Artists"
    | "Search"
    | "Favourites"
    | "Playlists"
    | "Queue"
    | "Shares"
    | "Admin";
  icon: IconName;
  admin?: boolean;
  primary?: boolean;
}> = [
  { to: "/", label: "Albums", icon: "albums", primary: true },
  { to: "/artists", label: "Artists", icon: "artists" },
  { to: "/search", label: "Search", icon: "search", primary: true },
  { to: "/favourites", label: "Favourites", icon: "heart", primary: true },
  { to: "/playlists", label: "Playlists", icon: "playlists", primary: true },
  { to: "/queue", label: "Queue", icon: "queue", primary: true },
  { to: "/shares", label: "Shares", icon: "shares" },
  { to: "/admin", label: "Admin", icon: "admin", admin: true },
];

function Brand() {
  const { t } = useI18n();
  return (
    <Link className="brand" to="/" aria-label={t("nav.home")}>
      <span className="brand-mark" aria-hidden="true">
        <i />
        <i />
        <i />
        <i />
      </span>
      <span>WaveFlow</span>
      <small>{t("nav.server")}</small>
    </Link>
  );
}

function Navigation({ mobile = false }: { mobile?: boolean }) {
  const user = currentUser();
  const { t } = useI18n();
  const labels = {
    Albums: t("nav.albums"),
    Artists: t("nav.artists"),
    Search: t("nav.search"),
    Favourites: t("nav.favourites"),
    Playlists: t("nav.playlists"),
    Queue: t("nav.queue"),
    Shares: t("nav.shares"),
    Admin: t("nav.admin"),
  };
  return (
    <nav className={mobile ? "mobile-navigation" : "primary-navigation"}>
      {navigation
        .filter(
          (item) =>
            (!item.admin || user?.role === "admin") &&
            (!mobile || item.primary),
        )
        .map((item) => (
          <Link
            key={item.to}
            to={item.to}
            aria-label={mobile ? labels[item.label] : undefined}
            activeOptions={{ exact: item.to === "/" }}
          >
            <Icon name={item.icon} />
            <span>{labels[item.label]}</span>
          </Link>
        ))}
    </nav>
  );
}

function Shell() {
  const navigate = useNavigate();
  const user = currentUser();
  const { t } = useI18n();
  return (
    <PlayerProvider>
      <a className="skip-link" href="#main-content">
        {t("nav.skip")}
      </a>
      <div className="shell">
        <aside className="sidebar">
          <Brand />
          <Navigation />
          <div className="sidebar-footer">
            <ThemePicker />
            <LanguagePicker />
            <div className="account-chip">
              <span aria-hidden="true">
                {user?.username.slice(0, 1).toUpperCase()}
              </span>
              <div>
                <strong>{user?.username}</strong>
                <small>{user?.role}</small>
              </div>
            </div>
            <button
              type="button"
              className="nav-action"
              onClick={async () => {
                await logout();
                await navigate({ to: "/login" });
              }}
            >
              <Icon name="logout" />
              {t("nav.signOut")}
            </button>
          </div>
        </aside>
        <header className="mobile-header">
          <Brand />
          <ThemePicker />
          <LanguagePicker />
        </header>
        <main id="main-content" tabIndex={-1}>
          <Outlet />
        </main>
        <Navigation mobile />
        <PlayerBar />
      </div>
    </PlayerProvider>
  );
}

const rootRoute = createRootRoute({
  component: Outlet,
  notFoundComponent: NotFoundPage,
});

const loginRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/login",
  component: LoginPage,
});

/** Everything below requires a session; unauthenticated visitors land on /login. */
const authedRoute = createRoute({
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

const albumsRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/",
  component: AlbumsPage,
});

const albumRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/albums/$albumId",
  component: function AlbumRoute() {
    const { albumId } = albumRoute.useParams();
    return <AlbumPage albumId={albumId} />;
  },
});

const artistsRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/artists",
  component: ArtistsPage,
});

const artistRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/artists/$artistId",
  component: function ArtistRoute() {
    const { artistId } = artistRoute.useParams();
    return <ArtistPage artistId={artistId} />;
  },
});

const authorizeRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/authorize",
  component: AuthorizePage,
});

const searchRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/search",
  component: SearchPage,
});

const favoritesRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/favourites",
  component: FavoritesPage,
});

const playlistsRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/playlists",
  component: PlaylistsPage,
});

const queueRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/queue",
  component: QueuePage,
});

const sharesRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/shares",
  component: SharesPage,
});

const adminRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/admin",
  beforeLoad: () => {
    if (currentUser()?.role !== "admin") throw redirect({ to: "/" });
  },
  component: AdminPage,
});

const routeTree = rootRoute.addChildren([
  loginRoute,
  authedRoute.addChildren([
    albumsRoute,
    albumRoute,
    artistsRoute,
    artistRoute,
    searchRoute,
    favoritesRoute,
    playlistsRoute,
    queueRoute,
    sharesRoute,
    adminRoute,
    authorizeRoute,
  ]),
]);

const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

const container = document.getElementById("root");
if (!container) throw new Error("missing #root");

createRoot(container).render(
  <StrictMode>
    <PreferencesProvider>
      <I18nProvider>
        <RouterProvider router={router} />
      </I18nProvider>
    </PreferencesProvider>
  </StrictMode>,
);
