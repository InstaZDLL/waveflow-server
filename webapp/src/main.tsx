import { QueryClientProvider } from "@tanstack/react-query";
import { createRouter, RouterProvider } from "@tanstack/react-router";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { forgetOnSessionChange, onSessionOwner } from "./api";
import { createQueryClient } from "./api/query-client";
import { routeTree } from "./app/route-tree";
import {
  claimOffline,
  forgetKnownOffline,
  registerServiceWorker,
} from "./lib/offline";
import { I18nProvider } from "./state/i18n";
import { PreferencesProvider } from "./state/preferences";
import "./styles/styles.css";
import "./styles/refresh.css";
// The Fluid Editorial faces, which every theme's rules now name: the
// structure in fluid.css is shared, and only its material is per theme.
import "@fontsource-variable/dm-sans/opsz.css";
import "@fontsource/playfair-display/400-italic.css";
import "./styles/fluid.css";

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
