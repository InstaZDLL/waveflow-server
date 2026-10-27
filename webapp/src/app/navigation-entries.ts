// The rail's entries: their groups, labels, icons and addresses.
import type { IconName } from "../components/layout/icons";
import type { TranslationKey } from "../state/i18n";

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
export const NAVIGATION_GROUPS = ["pinned", "library", "listen"] as const;
type NavigationGroup = (typeof NAVIGATION_GROUPS)[number] | "footer";

export const GROUP_LABELS: Record<
  Exclude<NavigationGroup, "pinned" | "footer">,
  TranslationKey
> = {
  library: "nav.group.library",
  listen: "nav.group.listen",
};

export const navigation: Array<{
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

export type NavigationTarget = (typeof navigation)[number]["to"];
