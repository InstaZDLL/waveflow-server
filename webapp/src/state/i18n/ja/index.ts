// The Japanese strings, assembled from the same domains as the English ones.
import type { TranslationKey, TranslationValue } from "..";
import { admin } from "./admin";
import { attention } from "./attention";
import { clients } from "./clients";
import { common } from "./common";
import { folders } from "./folders";
import { library } from "./library";
import { login } from "./login";
import { nav } from "./nav";
import { player } from "./player";
import { playlists } from "./playlists";
import { podcasts } from "./podcasts";
import { preferences } from "./preferences";
import { quality } from "./quality";
import { scrobbling } from "./scrobbling";
import { smart } from "./smart";
import { stats } from "./stats";
import { track } from "./track";

export const ja: Record<TranslationKey, TranslationValue> = {
  ...nav,
  ...preferences,
  ...common,
  ...library,
  ...clients,
  ...quality,
  ...track,
  ...login,
  ...playlists,
  ...player,
  ...admin,
  ...attention,
  ...folders,
  ...scrobbling,
  ...stats,
  ...smart,
  ...podcasts,
};
