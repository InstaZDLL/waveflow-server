// The listening routes: now playing, the queue, random, radio, podcasts.
import { createRoute } from "@tanstack/react-router";

import { PlayingPage } from "../../pages/playing-page";
import { PodcastsPage } from "../../pages/podcasts-page";
import { QueuePage } from "../../pages/queue-page";
import { RadioPage } from "../../pages/radio-page";
import { RandomPage } from "../../pages/random-page";
import { authedRoute } from "./root";

export const randomRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/random",
  component: RandomPage,
});

export const playingRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/playing",
  component: PlayingPage,
});

export const queueRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/queue",
  component: QueuePage,
});

export const radioRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/radio",
  staticData: { layout: "workspace" },
  component: RadioPage,
});

export const podcastsRoute = createRoute({
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
