import { QueryClientProvider } from "@tanstack/react-query";
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
import { type RefObject, StrictMode, useEffect, useRef } from "react";
import { createRoot } from "react-dom/client";

import {
  currentUser,
  ensureSession,
  forgetOnSessionChange,
  logout,
} from "./api/api";
import { createQueryClient } from "./api/query-client";
import { Icon, type IconName } from "./components/icons";
import { Waiting } from "./components/page-shell";
import { TrackEditorPage } from "./components/track-editor";
import { dismissSplash } from "./lib/splash";
import {
  AdminPage,
  AlbumPage,
  AlbumsPage,
  ArtistPage,
  ArtistsPage,
  AuthorizePage,
  FavoritesPage,
  GenrePage,
  GenresPage,
  HistoryPage,
  LoginPage,
  NotFoundPage,
  PlaylistsPage,
  QueuePage,
  RandomPage,
  SearchPage,
  SharesPage,
  SongsPage,
} from "./pages/pages";
import { PlayingPage } from "./pages/playing-page";
import { RadioPage } from "./pages/radio-page";
import { ScrobblingPage } from "./pages/scrobbling-page";
import { SettingsPage } from "./pages/settings-page";
import { UploadPage } from "./pages/upload-page";
import { PlayerBar, PlayerProvider } from "./player/player";
import { I18nProvider, type TranslationKey, useI18n } from "./state/i18n";
import {
  LibraryPicker,
  LibraryScopeProvider,
  mayUploadTo,
  useLibraryScope,
} from "./state/library-scope";
import { PreferencesProvider } from "./state/preferences";
import "./styles/styles.css";

const navigation: Array<{
  to:
    | "/"
    | "/artists"
    | "/songs"
    | "/genres"
    | "/search"
    | "/favourites"
    | "/history"
    | "/random"
    | "/playing"
    | "/playlists"
    | "/queue"
    | "/shares"
    | "/radio"
    | "/settings"
    | "/settings/scrobbling"
    | "/upload"
    | "/admin";
  labelKey: TranslationKey;
  icon: IconName;
  admin?: boolean;
  /** Shown only where the active library takes files from this account. */
  upload?: boolean;
  primary?: boolean;
}> = [
  { to: "/", labelKey: "nav.albums", icon: "albums", primary: true },
  { to: "/artists", labelKey: "nav.artists", icon: "artists" },
  { to: "/songs", labelKey: "nav.songs", icon: "songs" },
  { to: "/genres", labelKey: "nav.genres", icon: "genres" },
  { to: "/search", labelKey: "nav.search", icon: "search", primary: true },
  { to: "/playing", labelKey: "nav.playing", icon: "lyrics", primary: true },
  { to: "/random", labelKey: "nav.random", icon: "random" },
  { to: "/history", labelKey: "nav.history", icon: "history" },
  {
    to: "/favourites",
    labelKey: "nav.favourites",
    icon: "heart",
    primary: true,
  },
  {
    to: "/playlists",
    labelKey: "nav.playlists",
    icon: "playlists",
    primary: true,
  },
  { to: "/queue", labelKey: "nav.queue", icon: "queue", primary: true },
  { to: "/shares", labelKey: "nav.shares", icon: "shares" },
  { to: "/radio", labelKey: "nav.radio", icon: "radio" },
  { to: "/settings", labelKey: "nav.settings", icon: "admin" },
  {
    to: "/settings/scrobbling",
    labelKey: "nav.scrobbling",
    icon: "scrobbling",
  },
  { to: "/upload", labelKey: "nav.upload", icon: "upload", upload: true },
  { to: "/admin", labelKey: "nav.admin", icon: "admin", admin: true },
];

function Brand() {
  const { t } = useI18n();
  return (
    <Link className="brand" to="/" aria-label={t("nav.home")}>
      <img className="brand-mark" src="/logo.svg" alt="" aria-hidden="true" />
      <span>WaveFlow</span>
      <small>{t("nav.server")}</small>
    </Link>
  );
}

/**
 * Whether this account may see an entry at all.
 *
 * The two flags are about authority and not about width, so they apply
 * wherever the entry is rendered — bar or sheet. `primary` is the separate
 * question of what fits on a phone.
 */
function permitted(
  item: (typeof navigation)[number],
  role: string | undefined,
  library: Parameters<typeof mayUploadTo>[0],
): boolean {
  return (
    (!item.admin || role === "admin") && (!item.upload || mayUploadTo(library))
  );
}

function Navigation({ mobile = false }: { mobile?: boolean }) {
  const user = currentUser();
  const { active } = useLibraryScope();
  const { t } = useI18n();
  const visible = navigation.filter((item) =>
    permitted(item, user?.role, active),
  );
  // On a phone the bar holds the six things somebody reaches for while
  // listening. Everything else used to be reachable by typing its address and
  // by nothing else — the sidebar that carries it is `display: none` below
  // 820px — so the seventh slot opens the rest rather than promoting one of
  // it. Promoting one would have made the others more invisible by contrast.
  const overflow = mobile ? visible.filter((item) => !item.primary) : [];
  const sheet = useRef<HTMLDialogElement>(null);
  return (
    <>
      <nav className={mobile ? "mobile-navigation" : "primary-navigation"}>
        {visible
          .filter((item) => !mobile || item.primary)
          .map((item) => (
            <Link
              key={item.to}
              to={item.to}
              aria-label={mobile ? t(item.labelKey) : undefined}
              activeOptions={{ exact: item.to === "/" }}
            >
              <Icon name={item.icon} />
              <span>{t(item.labelKey)}</span>
            </Link>
          ))}
        {overflow.length > 0 ? (
          <button
            type="button"
            className="more-button"
            aria-label={t("nav.more")}
            onClick={() => sheet.current?.showModal()}
          >
            <Icon name="more" />
            <span>{t("nav.more")}</span>
          </button>
        ) : null}
      </nav>
      {/* Beside the bar and not inside it. The sheet's own links would
          otherwise count as the bar's, and a closed `<dialog>` is
          `display: none` — so a test measuring how many rows the bar occupies
          would read a zero-sized child as sitting on the first one, and could
          not see a bar that had wrapped. */}
      {overflow.length > 0 ? (
        <MoreSheet sheet={sheet} entries={overflow} />
      ) : null}
    </>
  );
}

/**
 * What the seventh slot opens.
 *
 * A native `<dialog>` opened with `showModal`, because that is where the focus
 * trap, the Escape key and the inertness of the page behind come from. Writing
 * those by hand is how an accessibility gate starts failing, and this one is
 * measured — the suite runs axe over the sheet while it is open.
 *
 * Rendered only when there is something behind it: an account that may neither
 * upload nor administer still has six entries hidden, but a listing with
 * nothing in it would be a control that does nothing.
 */
function MoreSheet({
  sheet,
  entries,
}: {
  sheet: RefObject<HTMLDialogElement | null>;
  entries: typeof navigation;
}) {
  const { t } = useI18n();

  return (
    <>
      {/* `onClick` on the dialog itself catches the backdrop: a click landing
          on the element rather than on anything inside it is a click outside
          the sheet, which is the only thing the backdrop can be.

          The keyboard equivalent of dismissing by the backdrop is Escape, and
          `<dialog>` already answers it — which is the whole reason the element
          was chosen over a hand-built overlay. An `onKeyDown` here would
          duplicate a behaviour the platform provides, and a second closer is a
          second thing to get out of step. */}
      {/* biome-ignore lint/a11y/useKeyWithClickEvents: dismissing by keyboard is Escape, which <dialog> handles natively */}
      <dialog
        ref={sheet}
        className="more-sheet"
        aria-label={t("nav.moreTitle")}
        onClick={(event) => {
          if (event.target === sheet.current) sheet.current?.close();
        }}
      >
        <header>
          <div>
            <h2>{t("nav.moreTitle")}</h2>
            <p className="muted">{t("nav.moreDetail")}</p>
          </div>
          <button
            type="button"
            aria-label={t("nav.moreClose")}
            onClick={() => sheet.current?.close()}
          >
            <Icon name="close" />
          </button>
        </header>
        <nav>
          {entries.map((item) => (
            <Link
              key={item.to}
              to={item.to}
              // Closed on the way out, not on arrival: the sheet would
              // otherwise stay open over the page it just navigated to.
              onClick={() => sheet.current?.close()}
              activeOptions={{ exact: item.to === "/" }}
            >
              <Icon name={item.icon} />
              <span>{t(item.labelKey)}</span>
            </Link>
          ))}
        </nav>
      </dialog>
    </>
  );
}

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
function Shell() {
  const navigate = useNavigate();
  const user = currentUser();
  const { t } = useI18n();
  return (
    <LibraryScopeProvider>
      <PlayerProvider>
        <a className="skip-link" href="#main-content">
          {t("nav.skip")}
        </a>
        <div className="shell">
          <aside className="sidebar">
            <Brand />
            <LibraryPicker />
            <Navigation />
            {/* The theme and the language used to sit here, under thirteen
                navigation entries. They are settings and not destinations, so
                they live on the Settings page now — and only there, because a
                copy left behind would give two ways to ask one question. */}
            <div className="sidebar-footer">
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
                  try {
                    await logout();
                  } catch {
                    // logout always clears local session state in its finally
                    // block; an unavailable server must not trap the user here.
                  }
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
            <LibraryPicker />
          </header>
          <main id="main-content" tabIndex={-1}>
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
function RootFrame() {
  return (
    <>
      <SplashGate />
      <Outlet />
    </>
  );
}

const rootRoute = createRootRoute({
  component: RootFrame,
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

const songsRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/songs",
  component: SongsPage,
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

const genresRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/genres",
  component: GenresPage,
});

const genreRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/genres/$genre",
  component: function GenreRoute() {
    const { genre } = genreRoute.useParams();
    return <GenrePage genre={genre} />;
  },
});

const historyRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/history",
  component: HistoryPage,
});

const randomRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/random",
  component: RandomPage,
});

const playingRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/playing",
  component: PlayingPage,
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

const radioRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/radio",
  component: RadioPage,
});

// The path is the server's, not a preference. `lastfm_callback` redirects
// here when a journey completes, so spelling it any other way puts the last
// step of that journey on the not-found page.
const settingsRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/settings",
  component: SettingsPage,
});

const scrobblingRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/settings/scrobbling",
  component: ScrobblingPage,
});

const adminRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/admin",
  beforeLoad: () => {
    if (currentUser()?.role !== "admin") throw redirect({ to: "/" });
  },
  component: AdminPage,
});

const uploadRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/upload",
  component: UploadPage,
});

const trackEditRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/tracks/$trackId/edit",
  component: function TrackEditRoute() {
    const { trackId } = trackEditRoute.useParams();
    return <TrackEditorPage trackId={trackId} />;
  },
});

const routeTree = rootRoute.addChildren([
  loginRoute,
  authedRoute.addChildren([
    albumsRoute,
    albumRoute,
    songsRoute,
    artistsRoute,
    artistRoute,
    genresRoute,
    genreRoute,
    historyRoute,
    randomRoute,
    playingRoute,
    searchRoute,
    favoritesRoute,
    playlistsRoute,
    queueRoute,
    sharesRoute,
    radioRoute,
    settingsRoute,
    scrobblingRoute,
    adminRoute,
    authorizeRoute,
    trackEditRoute,
    uploadRoute,
  ]),
]);

/**
 * A navigation that is drawn rather than instantaneous.
 *
 * The complaint the session recorded was that pages "load too fast to be
 * real", and the cause is the absence of a drawn transition rather than the
 * speed. `defaultViewTransition` hands that to the browser, which holds a
 * snapshot of the screen being left and cross-fades it into the one arriving
 * — **an actual cross-fade**, where fading the new screen in over nothing is
 * only half of one.
 *
 * It replaced a hand-written version of the same idea, and the comparison is
 * the argument: that one animated the incoming frame in a layout effect,
 * which meant deciding whether to key the frame on the path (it remounted the
 * screen, and a screen that mounts twice asks the server twice), whether the
 * effect ran before the paint (after it, the screen flashed at full opacity
 * first), and how to hold the outgoing view at all (it could not — the router
 * had already unmounted it). None of those questions exist here.
 *
 * `startViewTransition` is absent in older browsers; `router-core` checks for
 * it and navigates plainly when it is missing. The duration lives in
 * `styles.css`, where a `::view-transition` pseudo-element can be reached.
 *
 * **Reduced motion switches it off rather than animating it to nothing.** A
 * transition started and then neutralised in CSS has to win a cascade against
 * the user agent on a pseudo-element that exists only while it runs; not
 * starting one has nothing to win. Read once, at startup, so a reader who
 * changes the setting mid-session sees it after a reload — which is why the
 * media query in `styles.css` stays as well.
 *
 * The first load is untouched: a view transition is a thing between two
 * screens, and there is only one.
 */
const router = createRouter({
  routeTree,
  defaultViewTransition: !window.matchMedia("(prefers-reduced-motion: reduce)")
    .matches,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

const container = document.getElementById("root");
if (!container) throw new Error("missing #root");

// One client for the life of the document. Built here rather than at module
// scope so nothing holds a cache of somebody else's answers if this module is
// ever imported by a test.
const queryClient = createQueryClient();

// Emptied whenever the session changes. Everything the cache holds was answered
// for one account — a catalogue is scoped by membership, and an administrator's
// answers are scoped by more than that — so carrying it across a sign-out would
// show the next person what the last one could see.
forgetOnSessionChange(() => queryClient.clear());

createRoot(container).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <PreferencesProvider>
        <I18nProvider>
          <RouterProvider router={router} />
        </I18nProvider>
      </PreferencesProvider>
    </QueryClientProvider>
  </StrictMode>,
);
