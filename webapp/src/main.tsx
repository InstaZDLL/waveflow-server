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
import {
  AdminPage,
  AlbumPage,
  AlbumsPage,
  ArtistPage,
  ArtistsPage,
  AuthorizePage,
  FavoritesPage,
  LoginPage,
  PlaylistsPage,
  QueuePage,
  SearchPage,
  SharesPage,
} from "./pages";
import { PlayerBar, PlayerProvider } from "./player";
import "./styles.css";

/**
 * Renders the authenticated application shell with navigation, routed content, and the player bar.
 */
function Shell() {
  const navigate = useNavigate();
  const user = currentUser();
  return (
    <PlayerProvider>
      <div className="shell">
        <nav>
          <span className="brand">WaveFlow</span>
          <Link to="/">Albums</Link>
          <Link to="/artists">Artists</Link>
          <Link to="/search">Search</Link>
          <Link to="/favourites">Favourites</Link>
          <Link to="/playlists">Playlists</Link>
          <Link to="/queue">Queue</Link>
          <Link to="/shares">Shares</Link>
          {user?.role === "admin" ? <Link to="/admin">Admin</Link> : null}
          <button
            type="button"
            className="link"
            onClick={async () => {
              await logout();
              await navigate({ to: "/login" });
            }}
          >
            Sign out
          </button>
        </nav>
        <main>
          <Outlet />
        </main>
        <PlayerBar />
      </div>
    </PlayerProvider>
  );
}

const rootRoute = createRootRoute({ component: Outlet });

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
    <RouterProvider router={router} />
  </StrictMode>,
);
