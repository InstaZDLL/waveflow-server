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
  useMatches,
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
  onSessionOwner,
} from "./api";
import {
  albumsQuery,
  artistsQuery,
  favoriteTracksQuery,
  genresQuery,
  historyTracksQuery,
  ownApiTokensQuery,
  ownSubsonicCredentialQuery,
  playlistsQuery,
  podcastsQuery,
  radioStationsQuery,
  songsQuery,
  statsQuery,
} from "./api/queries";
import { createQueryClient } from "./api/query-client";
import { CoverLight } from "./components/cover-light";
import { Icon, type IconName } from "./components/icons";
import { Waiting } from "./components/page-shell";
import { TrackEditorPage } from "./components/track-editor";
import { localDayKey } from "./lib/local-day";
import {
  claimOffline,
  forgetKnownOffline,
  registerServiceWorker,
} from "./lib/offline";
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
import {
  MaintenanceHub,
  MaintenanceIndex,
  maintenanceSections,
} from "./pages/maintenance-hub";
import { MissingPage } from "./pages/missing-page";
import { MixPage } from "./pages/mix-page";
import { NotFoundPage } from "./pages/not-found-page";
import { OfflinePage } from "./pages/offline-page";
import { PlayingPage } from "./pages/playing-page";
import { PlaylistsPage } from "./pages/playlists-page";
import { PodcastsPage } from "./pages/podcasts-page";
import { isQualityVerdict, QualityPage } from "./pages/quality-page";
import { QueuePage } from "./pages/queue-page";
import { RadioPage } from "./pages/radio-page";
import { RandomPage } from "./pages/random-page";
import { ScrobblingPage } from "./pages/scrobbling-page";
import { SearchPage } from "./pages/search-page";
import { SettingsHub } from "./pages/settings-hub";
import { PlaybackSettingsPage, SettingsPage } from "./pages/settings-page";
import { SharePage } from "./pages/share-page";
import { SharesPage } from "./pages/shares-page";
import { SongsPage } from "./pages/songs-page";
import { StatsPage } from "./pages/stats-page";
import { UploadPage } from "./pages/upload-page";
import { PlayerBar, PlayerProvider } from "./player/player";
import { I18nProvider, type TranslationKey, useI18n } from "./state/i18n";
import {
  LibraryPicker,
  LibraryScopeProvider,
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
 *
 * Settings is not among them. It sits in the rail's foot, beside the account
 * it belongs to and above Sign out, where it stays in view however long the
 * rail above it grows. It used to head a fourth group, "Account", of five
 * entries; those are the Settings page's own sections now.
 */
const NAVIGATION_GROUPS = ["pinned", "library", "listen"] as const;
type NavigationGroup = (typeof NAVIGATION_GROUPS)[number] | "footer";

const GROUP_LABELS: Record<
  Exclude<NavigationGroup, "pinned" | "footer">,
  TranslationKey
> = {
  library: "nav.group.library",
  listen: "nav.group.listen",
};

const navigation: Array<{
  to:
    | "/"
    | "/artists"
    | "/songs"
    | "/folders"
    | "/maintenance"
    | "/genres"
    | "/search"
    | "/favourites"
    | "/history"
    | "/stats"
    | "/random"
    | "/playing"
    | "/playlists"
    | "/queue"
    | "/offline"
    | "/radio"
    | "/podcasts"
    | "/settings";
  labelKey: TranslationKey;
  icon: IconName;
  /**
   * Shown only when at least one of Maintenance's sections is this account's
   * to use in the active library; each keeps the rule its own entry had.
   */
  maintenance?: boolean;
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
  // One entry where three were: "Needs attention", "Missing files" and
  // "Quality" are the sections of Maintenance, each behind its own rule.
  {
    to: "/maintenance",
    labelKey: "nav.maintenance",
    icon: "maintenance",
    maintenance: true,
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
    to: "/stats",
    labelKey: "nav.stats",
    icon: "stats",
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
  {
    to: "/offline",
    labelKey: "nav.offline",
    icon: "download",
    group: "listen",
  },
  { to: "/radio", labelKey: "nav.radio", icon: "radio", group: "listen" },
  {
    to: "/podcasts",
    labelKey: "nav.podcasts",
    icon: "podcasts",
    group: "listen",
  },
  // One entry for everything about the account, where five used to be: the
  // sections behind it (shares, scrobbling, upload, administration) are the
  // Settings page's own menu now, and each keeps its gate there.
  {
    to: "/settings",
    labelKey: "nav.settings",
    icon: "admin",
    group: "footer",
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
    case "/stats":
      await client.ensureQueryData(statsQuery("30d", localDayKey()));
      break;
    case "/favourites":
      await client.ensureQueryData(favoriteTracksQuery());
      break;
    case "/playlists":
      await client.ensureQueryData(playlistsQuery());
      break;
    case "/radio":
      await client.ensureQueryData(radioStationsQuery());
      break;
    case "/podcasts":
      await client.ensureQueryData(podcastsQuery());
      break;
    // The section the entry opens on, which is Account.
    case "/settings":
      await Promise.all([
        client.ensureQueryData(ownApiTokensQuery()),
        client.ensureQueryData(ownSubsonicCredentialQuery()),
      ]);
      break;
    // These screens draw their frame immediately or read state already held
    // by the player. They have no page-wide answer to wait for.
    case "/search":
    case "/playing":
    case "/queue":
    // Read from this browser, not from the server.
    case "/offline":
    // The folder is in the address, and the rail links to the root: the
    // page draws its own frame and waits for its folder there.
    case "/folders":
    // Its lists are asked for once the page knows the active library.
    case "/maintenance":
      break;
    // A fresh draw on every ask, held for no time at all: a draw made here
    // is thrown away before the page mounts, and the page then draws again.
    // Waiting for it would double the wait for nothing.
    case "/random":
      break;
  }
}

/**
 * The horizontal logo, the brand guide's primary version, from the logo kit
 * unchanged: its wordmark is Inter outlined, which the guide asks for instead
 * of the name typed in an interface face. Two files, because the guide gives
 * one per background — the colour version on light, the dark one (mint and
 * white) on dark — and the theme is the reader's preference, not the system's,
 * so no `<picture>` media query can choose. Both are in the document and the
 * stylesheet shows the one that matches the root's `dark` class, which
 * `applyTheme` sets before the first render: nothing swaps after paint.
 *
 * The name has to contain the words on screen, in order (WCAG 2.5.3, which
 * axe checks as `label-content-name-mismatch`): someone who says "click
 * WaveFlow server" to voice control must land on this link, and "WaveFlow
 * server, home" is what they would read off the logo and the label beside it.
 */
function Brand() {
  const { t } = useI18n();
  return (
    <Link className="brand" to="/" aria-label={t("nav.home")}>
      <img
        className="brand-logo brand-logo-light"
        src="/brand/waveflow-logo-horizontal-color.svg"
        alt=""
        aria-hidden="true"
        width={131}
        height={32}
      />
      <img
        className="brand-logo brand-logo-dark"
        src="/brand/waveflow-logo-horizontal-dark.svg"
        alt=""
        aria-hidden="true"
        width={131}
        height={32}
      />
      <small>{t("nav.server")}</small>
    </Link>
  );
}

/**
 * Whether this account may see an entry at all.
 *
 * The flags are about authority and not about width, so they apply wherever
 * the entry is rendered — bar or sheet. `primary` is the separate question of
 * what fits on a phone. The upload and administration gates moved with their
 * entries, onto the Settings page's section menu.
 */
function permitted(
  item: (typeof navigation)[number],
  role: string | undefined,
  library: Parameters<typeof maintenanceSections>[0],
): boolean {
  return !item.maintenance || maintenanceSections(library, role).length > 0;
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

  const entry = (item: (typeof navigation)[number], className?: string) => (
    <Link
      key={item.to}
      to={item.to}
      className={className}
      onClick={(event) => openTab(event, item.to)}
      aria-busy={pendingTab === item.to}
      aria-label={mobile ? t(item.labelKey) : undefined}
      // Settings is not exact: every section of the page is under it, and
      // the entry stays the current one in whichever is open.
      activeOptions={{ exact: item.to === "/" }}
    >
      <Icon name={item.icon} />
      <span>{t(item.labelKey)}</span>
    </Link>
  );

  return (
    <>
      <nav className={mobile ? "mobile-navigation" : "primary-navigation"}>
        {mobile
          ? visible.filter((item) => item.primary).map((item) => entry(item))
          : NAVIGATION_GROUPS.map((group) => {
              const entries = visible.filter((item) => item.group === group);
              if (entries.length === 0) return null;
              return (
                <div className="nav-group" data-group={group} key={group}>
                  {group === "pinned" ? null : (
                    <span className="nav-label">{t(GROUP_LABELS[group])}</span>
                  )}
                  {entries.map((item) => entry(item))}
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
      {mobile ? null : (
        // In the same component as the rail rather than in the shell, so a
        // click on Settings and a click on a rail entry share one pending tab:
        // the later one cancels the earlier, as two rail entries do.
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
          {visible
            .filter((item) => item.group === "footer")
            .map((item) => entry(item, "nav-action"))}
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
      )}
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

/**
 * What a share link opens (RFC-020 decision 3). Beside the sign-in page and
 * not under `authed`: the person a link is sent to has no account, and a
 * session check would send them to a sign-in they cannot complete. The server
 * hands this shell out only at `/share/{token}`, to a browser asking for HTML.
 */
const shareRoute = createRoute({
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
const offlineRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/offline",
  component: OfflinePage,
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

// RFC-024: this account's listening, and the mixes drawn from it.
const statsRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/stats",
  component: StatsPage,
});

const mixRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/mixes/$mixId",
  component: function MixRoute() {
    const { mixId } = mixRoute.useParams();
    return <MixPage mixId={mixId} />;
  },
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

const radioRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/radio",
  staticData: { layout: "workspace" },
  component: RadioPage,
});

const podcastsRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/podcasts",
  staticData: { layout: "workspace" },
  // The channel whose episodes are open, so Back and a reload keep it.
  validateSearch: (search: Record<string, unknown>): { channel?: string } => ({
    channel: typeof search.channel === "string" ? search.channel : undefined,
  }),
  component: function PodcastsRoute() {
    const { channel } = podcastsRoute.useSearch();
    return <PodcastsPage channel={channel} />;
  },
});

/**
 * Settings, and everything about the account behind its one rail entry.
 *
 * A frame rather than a page: the section menu over whichever section is
 * open. Each section is its own route, so it can be linked to and Back walks
 * between them; `/settings` itself is the first section, Account.
 */
const settingsRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/settings",
  component: SettingsHub,
});

const accountSettingsRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/",
  component: SettingsPage,
});

const playbackSettingsRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/playback",
  component: PlaybackSettingsPage,
});

// The path is the server's, not a preference. `lastfm_callback` redirects
// here when a journey completes, so spelling it any other way puts the last
// step of that journey on the not-found page.
const scrobblingRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/scrobbling",
  component: ScrobblingPage,
});

const sharesRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/shares",
  component: SharesPage,
});

const uploadRoute = createRoute({
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

const adminRoute = createRoute({
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
const legacySharesRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/shares",
  beforeLoad: () => {
    throw redirect({ to: "/settings/shares", replace: true });
  },
});

const legacyUploadRoute = createRoute({
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

const legacyAdminRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/admin",
  beforeLoad: () => {
    throw redirect({ to: "/settings/admin", replace: true });
  },
});

// The folder is a search parameter rather than a path segment: a folder's
// name may hold any character a file system allows, `$` and `?` included,
// and a parameter carries it whole.
const foldersRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/folders",
  staticData: { layout: "workspace" },
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

/**
 * Maintenance: "Needs attention", "Missing files" and "Quality" behind one
 * rail entry, as Settings holds the account's pages. `/maintenance` itself
 * opens the first section the account may use.
 */
const maintenanceRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/maintenance",
  // Every section under the hub inherits it: a child route's match carries
  // its own static data, but the hub's match is in the list beside it.
  staticData: { layout: "workspace" },
  component: MaintenanceHub,
});

const maintenanceIndexRoute = createRoute({
  getParentRoute: () => maintenanceRoute,
  path: "/",
  component: MaintenanceIndex,
});

const validateAttentionSearch = (search: Record<string, unknown>) => ({
  category: isAttentionCategory(search.category) ? search.category : undefined,
});

const validateQualitySearch = (search: Record<string, unknown>) => ({
  verdict: isQualityVerdict(search.verdict) ? search.verdict : undefined,
});

// The open list is a search parameter, so a list can be linked to and the
// back button walks between them.
const attentionRoute = createRoute({
  getParentRoute: () => maintenanceRoute,
  path: "/attention",
  validateSearch: validateAttentionSearch,
  component: function AttentionRoute() {
    const { category } = attentionRoute.useSearch();
    return <AttentionPage category={category} />;
  },
});

const missingRoute = createRoute({
  getParentRoute: () => maintenanceRoute,
  path: "/missing",
  component: MissingPage,
});

// The verdict filter is a search parameter, so a filtered list can be linked
// to, as the lists of "Needs attention" can.
const qualityRoute = createRoute({
  getParentRoute: () => maintenanceRoute,
  path: "/quality",
  validateSearch: validateQualitySearch,
  component: function QualityRoute() {
    const { verdict } = qualityRoute.useSearch();
    return <QualityPage verdict={verdict} />;
  },
});

// Their addresses before they moved under Maintenance, redirected as the
// Settings pages' are: the open list or verdict filter comes along, and the
// redirect replaces its own history entry.
const legacyAttentionRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/attention",
  validateSearch: validateAttentionSearch,
  beforeLoad: ({ search }) => {
    throw redirect({ to: "/maintenance/attention", search, replace: true });
  },
});

const legacyMissingRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/missing",
  beforeLoad: () => {
    throw redirect({ to: "/maintenance/missing", replace: true });
  },
});

const legacyQualityRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/quality",
  validateSearch: validateQualitySearch,
  beforeLoad: ({ search }) => {
    throw redirect({ to: "/maintenance/quality", search, replace: true });
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
  shareRoute,
  offlineRoute,
  authedRoute.addChildren([
    albumsRoute,
    albumRoute,
    songsRoute,
    foldersRoute,
    maintenanceRoute.addChildren([
      maintenanceIndexRoute,
      attentionRoute,
      missingRoute,
      qualityRoute,
    ]),
    legacyAttentionRoute,
    legacyMissingRoute,
    legacyQualityRoute,
    artistsRoute,
    artistRoute,
    genresRoute,
    genreRoute,
    historyRoute,
    statsRoute,
    mixRoute,
    randomRoute,
    playingRoute,
    searchRoute,
    favoritesRoute,
    playlistsRoute,
    queueRoute,
    radioRoute,
    podcastsRoute,
    settingsRoute.addChildren([
      accountSettingsRoute,
      playbackSettingsRoute,
      scrobblingRoute,
      sharesRoute,
      uploadRoute,
      adminRoute,
    ]),
    legacySharesRoute,
    legacyUploadRoute,
    legacyAdminRoute,
    authorizeRoute,
    trackEditRoute,
  ]),
]);

const router = createRouter({
  routeTree,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
  interface StaticDataRouteOption {
    /**
     * `workspace` for the pages that manage something rather than present
     * it — Radio, Podcasts, Folders and the Maintenance hub. They take more of
     * a wide window; the editorial pages keep the contained column.
     */
    layout?: "workspace";
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

// The shell and the kept tracks, for listening offline (RFC-022). Never an
// API response: see `public/sw.js`.
registerServiceWorker(import.meta.env.PROD);
// The kept tracks are one account's (RFC-022): a sign-out deletes them, and
// a different account signing in deletes the last one's before it sees them.
onSessionOwner((userId) => claimOffline(userId));
forgetOnSessionChange(forgetKnownOffline);

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
