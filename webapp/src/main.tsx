import {
  type QueryClient,
  QueryClientProvider,
  useQueryClient,
} from "@tanstack/react-query";
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
import {
  type MouseEvent,
  type RefObject,
  StrictMode,
  useEffect,
  useRef,
  useState,
} from "react";
import { createRoot } from "react-dom/client";

import {
  currentUser,
  ensureSession,
  forgetOnSessionChange,
  logout,
} from "./api";
import {
  administrationQuery,
  albumsQuery,
  artistsQuery,
  favoriteTracksQuery,
  genresQuery,
  historyTracksQuery,
  ownApiTokensQuery,
  ownSubsonicCredentialQuery,
  playlistsQuery,
  radioStationsQuery,
  scrobbleRowsQuery,
  sharesQuery,
  songsQuery,
  uncertainScrobblesQuery,
} from "./api/queries";
import { createQueryClient } from "./api/query-client";
import { CoverLight } from "./components/cover-light";
import { Icon, type IconName } from "./components/icons";
import { Waiting } from "./components/page-shell";
import { TrackEditorPage } from "./components/track-editor";
import { dismissSplash } from "./lib/splash";
import { AdminPage } from "./pages/admin-page";
import { AlbumPage } from "./pages/album-page";
import { AlbumsPage } from "./pages/albums-page";
import { ArtistPage } from "./pages/artist-page";
import { ArtistsPage } from "./pages/artists-page";
import { AttentionPage, isAttentionCategory } from "./pages/attention-page";
import { AuthorizePage } from "./pages/authorize-page";
import { FavoritesPage } from "./pages/favorites-page";
import { FoldersPage } from "./pages/folders-page";
import { GenrePage } from "./pages/genre-page";
import { GenresPage } from "./pages/genres-page";
import { HistoryPage } from "./pages/history-page";
import { LoginPage } from "./pages/login-page";
import { MissingPage } from "./pages/missing-page";
import { NotFoundPage } from "./pages/not-found-page";
import { PlayingPage } from "./pages/playing-page";
import { PlaylistsPage } from "./pages/playlists-page";
import { isQualityVerdict, QualityPage } from "./pages/quality-page";
import { QueuePage } from "./pages/queue-page";
import { RadioPage } from "./pages/radio-page";
import { RandomPage } from "./pages/random-page";
import { ScrobblingPage } from "./pages/scrobbling-page";
import { SearchPage } from "./pages/search-page";
import { SettingsPage } from "./pages/settings-page";
import { SharesPage } from "./pages/shares-page";
import { SongsPage } from "./pages/songs-page";
import { UploadPage } from "./pages/upload-page";
import { PlayerBar, PlayerProvider } from "./player/player";
import { I18nProvider, type TranslationKey, useI18n } from "./state/i18n";
import {
  LibraryPicker,
  LibraryScopeProvider,
  mayCorrectTracks,
  mayUploadTo,
  useLibraryScope,
} from "./state/library-scope";
import { PreferencesProvider } from "./state/preferences";
import "./styles/styles.css";
import "./styles/refresh.css";
// The Fluid Editorial faces, which every theme's rules now name: the
// structure in fluid.css is shared, and only its material is per theme.
import "@fontsource-variable/dm-sans/opsz.css";
import "@fontsource/playfair-display/400-italic.css";
import "./styles/fluid.css";

/**
 * The rail's sections, in the order it shows them.
 *
 * `pinned` is the four places reached for most, drawn as tiles two to a row;
 * the others carry a label.
 */
const NAVIGATION_GROUPS = ["pinned", "library", "listen", "account"] as const;
type NavigationGroup = (typeof NAVIGATION_GROUPS)[number];

const GROUP_LABELS: Record<
  Exclude<NavigationGroup, "pinned">,
  TranslationKey
> = {
  library: "nav.group.library",
  listen: "nav.group.listen",
  account: "nav.group.account",
};

const navigation: Array<{
  to:
    | "/"
    | "/artists"
    | "/songs"
    | "/folders"
    | "/attention"
    | "/missing"
    | "/quality"
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
  /** Shown only to the active library's owners and managers. */
  curate?: boolean;
  primary?: boolean;
  /** The rail section it is filed under. */
  group: NavigationGroup;
}> = [
  {
    to: "/",
    labelKey: "nav.albums",
    icon: "albums",
    primary: true,
    group: "pinned",
  },
  { to: "/artists", labelKey: "nav.artists", icon: "artists", group: "pinned" },
  { to: "/songs", labelKey: "nav.songs", icon: "songs", group: "pinned" },
  { to: "/genres", labelKey: "nav.genres", icon: "genres", group: "library" },
  {
    to: "/folders",
    labelKey: "nav.folders",
    icon: "folder",
    group: "library",
  },
  {
    to: "/attention",
    labelKey: "nav.attention",
    icon: "info",
    curate: true,
    group: "library",
  },
  {
    to: "/missing",
    labelKey: "nav.missing",
    icon: "inbox",
    curate: true,
    group: "library",
  },
  {
    to: "/quality",
    labelKey: "nav.quality",
    icon: "quality",
    group: "library",
  },
  {
    to: "/search",
    labelKey: "nav.search",
    icon: "search",
    primary: true,
    group: "pinned",
  },
  {
    to: "/playing",
    labelKey: "nav.playing",
    icon: "lyrics",
    primary: true,
    group: "listen",
  },
  { to: "/random", labelKey: "nav.random", icon: "random", group: "listen" },
  {
    to: "/history",
    labelKey: "nav.history",
    icon: "history",
    group: "library",
  },
  {
    to: "/favourites",
    labelKey: "nav.favourites",
    icon: "heart",
    primary: true,
    group: "library",
  },
  {
    to: "/playlists",
    labelKey: "nav.playlists",
    icon: "playlists",
    primary: true,
    group: "library",
  },
  {
    to: "/queue",
    labelKey: "nav.queue",
    icon: "queue",
    primary: true,
    group: "listen",
  },
  { to: "/shares", labelKey: "nav.shares", icon: "shares", group: "account" },
  { to: "/radio", labelKey: "nav.radio", icon: "radio", group: "listen" },
  {
    to: "/settings",
    labelKey: "nav.settings",
    icon: "admin",
    group: "account",
  },
  {
    to: "/settings/scrobbling",
    labelKey: "nav.scrobbling",
    icon: "scrobbling",
    group: "account",
  },
  {
    to: "/upload",
    labelKey: "nav.upload",
    icon: "upload",
    upload: true,
    group: "account",
  },
  {
    to: "/admin",
    labelKey: "nav.admin",
    icon: "admin",
    admin: true,
    group: "account",
  },
];

type NavigationTarget = (typeof navigation)[number]["to"];

/**
 * How long a tab switch holds the current screen while the next one loads.
 *
 * Long enough that an ordinary answer arrives first and the switch shows no
 * skeleton; short enough that a server slow to answer leaves the visitor on
 * the screen they asked for, with its wait, rather than on the old one.
 */
const TAB_WAIT_MS = 1500;

/** Have the destination's first answer ready before replacing the current tab. */
async function prepareTab(
  to: NavigationTarget,
  scope: string | undefined,
  client: QueryClient,
): Promise<void> {
  switch (to) {
    case "/":
      await client.ensureQueryData(albumsQuery("alphabeticalByName", scope));
      break;
    case "/artists":
      await client.ensureQueryData(artistsQuery(scope));
      break;
    case "/songs":
      await client.ensureInfiniteQueryData(
        songsQuery("alphabeticalByName", scope),
      );
      break;
    case "/genres":
      await client.ensureQueryData(genresQuery(scope));
      break;
    case "/history":
      await client.ensureQueryData(historyTracksQuery(50));
      break;
    case "/favourites":
      await client.ensureQueryData(favoriteTracksQuery());
      break;
    case "/playlists":
      await client.ensureQueryData(playlistsQuery());
      break;
    case "/shares":
      await client.ensureQueryData(sharesQuery());
      break;
    case "/radio":
      await client.ensureQueryData(radioStationsQuery());
      break;
    case "/settings":
      await Promise.all([
        client.ensureQueryData(ownApiTokensQuery()),
        client.ensureQueryData(ownSubsonicCredentialQuery()),
      ]);
      break;
    case "/settings/scrobbling":
      await Promise.all([
        client.ensureQueryData(scrobbleRowsQuery()),
        client.ensureQueryData(uncertainScrobblesQuery()),
      ]);
      break;
    case "/admin":
      await client.ensureQueryData(administrationQuery());
      break;
    // These screens draw their frame immediately or read state already held
    // by the player. They have no page-wide answer to wait for.
    case "/search":
    case "/playing":
    case "/queue":
    case "/upload":
    // The folder is in the address, and the rail links to the root: the
    // page draws its own frame and waits for its folder there.
    case "/folders":
    // Its lists are asked for once the page knows the active library.
    case "/attention":
    case "/missing":
    case "/quality":
      break;
    // A fresh draw on every ask, held for no time at all: a draw made here
    // is thrown away before the page mounts, and the page then draws again.
    // Waiting for it would double the wait for nothing.
    case "/random":
      break;
  }
}

/**
 * The name has to contain the words on screen, in order (WCAG 2.5.3, which
 * axe checks as `label-content-name-mismatch`): someone who says "click
 * WaveFlow server" to voice control must land on this link. The space between
 * the two pieces is load-bearing for the same reason — axe reads the visible
 * text by joining the children with nothing, so without it the label is the
 * one word "WaveFlowserver", which no name contains. The flex row drops the
 * space from the layout, so it changes nothing that is drawn.
 */
function Brand() {
  const { t } = useI18n();
  return (
    <Link className="brand" to="/" aria-label={t("nav.home")}>
      <img className="brand-mark" src="/logo.svg" alt="" aria-hidden="true" />
      <span>WaveFlow</span> <small>{t("nav.server")}</small>
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
    (!item.admin || role === "admin") &&
    (!item.upload || mayUploadTo(library)) &&
    (!item.curate || mayCorrectTracks(library ?? undefined))
  );
}

function Navigation({ mobile = false }: { mobile?: boolean }) {
  const user = currentUser();
  const { active } = useLibraryScope();
  const { t } = useI18n();
  const client = useQueryClient();
  const navigate = useNavigate();
  const [pendingTab, setPendingTab] = useState<NavigationTarget | null>(null);
  const navigationAttempt = useRef(0);
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

  function openTab(event: MouseEvent<HTMLAnchorElement>, to: NavigationTarget) {
    // Leave modified and middle clicks to the link, so opening a new tab keeps
    // the browser's normal behaviour. Only an ordinary in-app tab switch waits.
    if (
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.shiftKey ||
      event.altKey ||
      event.defaultPrevented
    ) {
      return;
    }
    if (window.location.pathname === to) {
      // A second click on the current tab cancels a destination still loading.
      // With nothing loading it is an ordinary link: it takes the tab back to
      // its plain address, `/playing?view=lyrics` to `/playing`.
      if (pendingTab === null) return;
      event.preventDefault();
      navigationAttempt.current += 1;
      setPendingTab(null);
      return;
    }
    event.preventDefault();
    const attempt = ++navigationAttempt.current;
    const origin = window.location.pathname + window.location.search;
    setPendingTab(to);
    void (async () => {
      try {
        // Waited for, but not for ever: past the deadline the destination
        // shows its own wait, which says more than a busy tab does.
        await Promise.race([
          prepareTab(to, active?.id, client),
          new Promise((resolve) => setTimeout(resolve, TAB_WAIT_MS)),
        ]);
      } catch {
        // The destination owns its error state. Navigate to it even when its
        // first request fails, rather than leaving the old tab stuck as busy.
      }
      if (attempt !== navigationAttempt.current) return;
      setPendingTab(null);
      // The visitor moved on while this loaded — a link in the page, Back —
      // and that move is the later decision. Arriving now would undo it.
      if (window.location.pathname + window.location.search !== origin) return;
      await navigate({ to });
    })();
  }

  const entry = (item: (typeof navigation)[number]) => (
    <Link
      key={item.to}
      to={item.to}
      onClick={(event) => openTab(event, item.to)}
      aria-busy={pendingTab === item.to}
      aria-label={mobile ? t(item.labelKey) : undefined}
      activeOptions={{
        exact: item.to === "/" || item.to === "/settings",
      }}
    >
      <Icon name={item.icon} />
      <span>{t(item.labelKey)}</span>
    </Link>
  );

  return (
    <>
      <nav className={mobile ? "mobile-navigation" : "primary-navigation"}>
        {mobile
          ? visible.filter((item) => item.primary).map(entry)
          : NAVIGATION_GROUPS.map((group) => {
              const entries = visible.filter((item) => item.group === group);
              if (entries.length === 0) return null;
              return (
                <div className="nav-group" data-group={group} key={group}>
                  {group === "pinned" ? null : (
                    <span className="nav-label">{t(GROUP_LABELS[group])}</span>
                  )}
                  {entries.map(entry)}
                </div>
              );
            })}
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
        <MoreSheet
          sheet={sheet}
          entries={overflow}
          pendingTab={pendingTab}
          onNavigate={openTab}
        />
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
  pendingTab,
  onNavigate,
}: {
  sheet: RefObject<HTMLDialogElement | null>;
  entries: typeof navigation;
  pendingTab: NavigationTarget | null;
  onNavigate: (
    event: MouseEvent<HTMLAnchorElement>,
    to: NavigationTarget,
  ) => void;
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
              aria-busy={pendingTab === item.to}
              // Closed on the way out, not on arrival: the sheet would
              // otherwise stay open over the page it just navigated to.
              onClick={(event) => {
                onNavigate(event, item.to);
                if (
                  event.button === 0 &&
                  !event.metaKey &&
                  !event.ctrlKey &&
                  !event.shiftKey &&
                  !event.altKey
                ) {
                  sheet.current?.close();
                }
              }}
              activeOptions={{
                exact: item.to === "/" || item.to === "/settings",
              }}
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
        <CoverLight />
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

// The folder is a search parameter rather than a path segment: a folder's
// name may hold any character a file system allows, `$` and `?` included,
// and a parameter carries it whole.
const foldersRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/folders",
  // The folder: absent or empty for the root, `unfiled` for the tracks filed
  // nowhere, otherwise a folder id. Optional, so the rail links to the root.
  validateSearch: (search: Record<string, unknown>): { folder?: string } => ({
    folder: typeof search.folder === "string" ? search.folder : undefined,
  }),
  component: function FoldersRoute() {
    const { folder } = foldersRoute.useSearch();
    return <FoldersPage folder={folder ?? ""} />;
  },
});

// The open list is a search parameter, so a list can be linked to and the
// back button walks between them.
const attentionRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/attention",
  validateSearch: (search: Record<string, unknown>) => ({
    category: isAttentionCategory(search.category)
      ? search.category
      : undefined,
  }),
  component: function AttentionRoute() {
    const { category } = attentionRoute.useSearch();
    return <AttentionPage category={category} />;
  },
});

const missingRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/missing",
  component: MissingPage,
});

// The verdict filter is a search parameter, so a filtered list can be linked
// to, as the lists of "Needs attention" can.
const qualityRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/quality",
  validateSearch: (search: Record<string, unknown>) => ({
    verdict: isQualityVerdict(search.verdict) ? search.verdict : undefined,
  }),
  component: function QualityRoute() {
    const { verdict } = qualityRoute.useSearch();
    return <QualityPage verdict={verdict} />;
  },
});

const uploadRoute = createRoute({
  getParentRoute: () => authedRoute,
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
    foldersRoute,
    attentionRoute,
    missingRoute,
    qualityRoute,
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

const router = createRouter({
  routeTree,
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
