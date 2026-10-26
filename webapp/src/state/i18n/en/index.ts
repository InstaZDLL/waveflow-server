// The English strings, one file per domain, in their original order.
import { browse } from "./browse";
import { folders } from "./folders";
import { library } from "./library";
import { maintenance } from "./maintenance";
import { playing } from "./playing";
import { podcasts } from "./podcasts";
import { quality } from "./quality";
import { scrobbling } from "./scrobbling";
import { settings } from "./settings";
import { shell } from "./shell";
import { smart } from "./smart";
import { stats } from "./stats";
import { track } from "./track";

export const en = {
  ...shell,
  ...settings,
  ...track,
  ...library,
  ...browse,
  ...quality,
  ...folders,
  ...playing,
  ...maintenance,
  ...scrobbling,
  ...stats,
  ...smart,
  ...podcasts,
};
