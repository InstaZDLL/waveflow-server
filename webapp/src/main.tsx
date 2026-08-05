import {
  createRootRoute,
  createRoute,
  createRouter,
  Link,
  Outlet,
  redirect,
  RouterProvider,
  useNavigate,
} from "@tanstack/react-router";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { logout, storedTokens } from "./api";
import {
  AlbumPage,
  AlbumsPage,
  ArtistPage,
  ArtistsPage,
  LoginPage,
  SearchPage,
} from "./pages";
import { PlayerBar, PlayerProvider } from "./player";
import "./styles.css";

function Shell() {
  const navigate = useNavigate();
  return (
    <div className="shell">
      <nav>
        <span className="brand">WaveFlow</span>
        <Link to="/">Albums</Link>
        <Link to="/artists">Artists</Link>
        <Link to="/search">Search</Link>
        <button
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
  beforeLoad: () => {
    if (!storedTokens()) throw redirect({ to: "/login" });
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

const searchRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/search",
  component: SearchPage,
});

const routeTree = rootRoute.addChildren([
  loginRoute,
  authedRoute.addChildren([
    albumsRoute,
    albumRoute,
    artistsRoute,
    artistRoute,
    searchRoute,
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
    <PlayerProvider>
      <RouterProvider router={router} />
    </PlayerProvider>
  </StrictMode>,
);
