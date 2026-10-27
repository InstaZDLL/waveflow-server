// Maintenance and its sections, and the addresses they had before it.
import { createRoute, redirect } from "@tanstack/react-router";

import {
  AttentionPage,
  isAttentionCategory,
} from "../../pages/maintenance/attention-page";
import {
  MaintenanceHub,
  MaintenanceIndex,
} from "../../pages/maintenance/maintenance-hub";
import { MissingPage } from "../../pages/maintenance/missing-page";
import {
  isQualityVerdict,
  QualityPage,
} from "../../pages/maintenance/quality-page";
import { authedRoute } from "./root";

/**
 * Maintenance: "Needs attention", "Missing files" and "Quality" behind one
 * rail entry, as Settings holds the account's pages. `/maintenance` itself
 * opens the first section the account may use.
 */
export const maintenanceRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/maintenance",
  // Every section under the hub inherits it: a child route's match carries
  // its own static data, but the hub's match is in the list beside it.
  staticData: { layout: "workspace" },
  component: MaintenanceHub,
});

export const maintenanceIndexRoute = createRoute({
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
export const attentionRoute = createRoute({
  getParentRoute: () => maintenanceRoute,
  path: "/attention",
  validateSearch: validateAttentionSearch,
  component: function AttentionRoute() {
    const { category } = attentionRoute.useSearch();
    return <AttentionPage category={category} />;
  },
});

export const missingRoute = createRoute({
  getParentRoute: () => maintenanceRoute,
  path: "/missing",
  component: MissingPage,
});

// The verdict filter is a search parameter, so a filtered list can be linked
// to, as the lists of "Needs attention" can.
export const qualityRoute = createRoute({
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
export const legacyAttentionRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/attention",
  validateSearch: validateAttentionSearch,
  beforeLoad: ({ search }) => {
    throw redirect({ to: "/maintenance/attention", search, replace: true });
  },
});

export const legacyMissingRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/missing",
  beforeLoad: () => {
    throw redirect({ to: "/maintenance/missing", replace: true });
  },
});

export const legacyQualityRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/quality",
  validateSearch: validateQualitySearch,
  beforeLoad: ({ search }) => {
    throw redirect({ to: "/maintenance/quality", search, replace: true });
  },
});
