// The brand, the rail and the phone's bar, and the tab switch that waits.
import { type QueryClient, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { type MouseEvent, useRef, useState } from "react";

import { currentUser, logout } from "../api";
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
} from "../api/queries";
import { Icon } from "../components/icons";
import { localDayKey } from "../lib/local-day";
import { maintenanceSections } from "../pages/maintenance-hub";
import { useI18n } from "../state/i18n";
import { useLibraryScope } from "../state/library-scope";
import { MoreSheet } from "./more-sheet";
import {
  GROUP_LABELS,
  NAVIGATION_GROUPS,
  type NavigationTarget,
  navigation,
} from "./navigation-entries";

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
export function Brand() {
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

export function Navigation({ mobile = false }: { mobile?: boolean }) {
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
