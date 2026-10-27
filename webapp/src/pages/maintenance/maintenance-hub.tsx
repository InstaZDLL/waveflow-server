import { Link, Navigate, Outlet } from "@tanstack/react-router";

import { currentUser, type Library } from "../../api";
import { PageHeader } from "../../components/layout/page-shell";
import { type TranslationKey, useI18n } from "../../state/i18n";
import {
  mayCorrectTracks,
  mayManageMissing,
  useLibraryScope,
} from "../../state/library-scope";

/**
 * Maintenance: the pages that put the library right rather than play it.
 *
 * "Needs attention", "Missing files" and "Quality" were three rail entries.
 * They are one now, built like Settings: a section menu over the open section,
 * one route each under `/maintenance`, and their old addresses redirect.
 *
 * Each section keeps the rule its rail entry had, so an account sees exactly
 * the sections it saw entries before — and the rail shows Maintenance only
 * when at least one of them is left.
 */

type Section = {
  to:
    | "/maintenance/attention"
    | "/maintenance/missing"
    | "/maintenance/quality";
  labelKey: TranslationKey;
  /** Whether this account may use the section in the active library. */
  visible: (library: Library | undefined, role: string | undefined) => boolean;
};

const SECTIONS: Section[] = [
  // Nothing on it is a listener's to decide (RFC-015).
  {
    to: "/maintenance/attention",
    labelKey: "nav.attention",
    visible: (library) => mayCorrectTracks(library),
  },
  // The library's owners and managers, and administrators whatever their
  // role in it (RFC-021).
  {
    to: "/maintenance/missing",
    labelKey: "nav.missing",
    visible: (library, role) => mayManageMissing(library, role),
  },
  // Every member reads the quality of the files (RFC-016).
  { to: "/maintenance/quality", labelKey: "nav.quality", visible: () => true },
];

/** The sections this account may use in this library, in the menu's order. */
export function maintenanceSections(
  library: Library | null | undefined,
  role: string | undefined,
): Section[] {
  return SECTIONS.filter((section) =>
    section.visible(library ?? undefined, role),
  );
}

function useVisibleSections(): Section[] {
  const { active } = useLibraryScope();
  return maintenanceSections(active, currentUser()?.role);
}

export function MaintenanceSections() {
  const { t } = useI18n();
  const sections = useVisibleSections();
  return (
    <nav className="section-menu" aria-label={t("maintenance.sections")}>
      {sections.map((section) => (
        <Link key={section.to} to={section.to} className="section-menu-link">
          {t(section.labelKey)}
        </Link>
      ))}
    </nav>
  );
}

/** The frame every section is drawn in: the menu, then the section. */
export function MaintenanceHub() {
  return (
    <>
      <MaintenanceSections />
      <Outlet />
    </>
  );
}

/**
 * `/maintenance` itself, which is where the rail entry points: it opens the
 * first section this account may use, replacing its own history entry so
 * Back does not land on an address that only sends the visitor forward.
 *
 * A component rather than a route redirect, because which section comes
 * first depends on the active library, which only the page knows.
 */
export function MaintenanceIndex() {
  const { t } = useI18n();
  const [first] = useVisibleSections();
  if (first) return <Navigate to={first.to} replace />;
  return (
    <section>
      <PageHeader title={t("nav.maintenance")} />
      <p className="muted">{t("maintenance.none")}</p>
    </section>
  );
}
