import { Link, Outlet } from "@tanstack/react-router";

import { currentUser } from "../../api";
import { SourceOffer } from "../../components/layout/source-offer";
import { type TranslationKey, useI18n } from "../../state/i18n";
import { mayUploadTo, useLibraryScope } from "../../state/library-scope";

/**
 * Settings, as one place with sections.
 *
 * The rail used to carry five entries under "Account" — Shares, Settings,
 * Scrobbling, Upload and Admin — and the rail scrolled. They are one entry now,
 * and this is what it opens: a section menu over whichever section is open.
 *
 * **Links, not ARIA tabs.** Each section is its own route, so it can be
 * bookmarked, opened in a new tab and walked back from; a tab widget would
 * promise arrow-key switching inside one page, which is not what happens. The
 * open section is marked by the router's `aria-current="page"`, and the
 * styling reads that rather than a class kept in step with it.
 *
 * **One list for every width.** A row of tabs on a desktop; on a phone the
 * same links wrap into a list two to a row (see the stylesheet). A `<select>`
 * that navigated on change was the other way to fit it, and it would move
 * the page under someone who is only arrowing through the options.
 */

type Section = {
  to:
    | "/settings"
    | "/settings/playback"
    | "/settings/scrobbling"
    | "/settings/shares"
    | "/settings/upload"
    | "/settings/admin";
  labelKey: TranslationKey;
  /** Shown only where the active library takes files from this account. */
  upload?: boolean;
  /** Shown only to administrators. */
  admin?: boolean;
};

const SECTIONS: Section[] = [
  { to: "/settings", labelKey: "settings.section.account" },
  { to: "/settings/playback", labelKey: "settings.playback" },
  { to: "/settings/scrobbling", labelKey: "nav.scrobbling" },
  { to: "/settings/shares", labelKey: "nav.shares" },
  // The gates the rail entries had, moved with them. They hide a section the
  // account cannot use; the routes still answer, and say why, when reached by
  // their address — the upload page names which of its two locks is shut, and
  // the administration route sends anyone else home.
  { to: "/settings/upload", labelKey: "nav.upload", upload: true },
  { to: "/settings/admin", labelKey: "nav.admin", admin: true },
];

export function SettingsSections() {
  const { t } = useI18n();
  const { active } = useLibraryScope();
  const role = currentUser()?.role;
  const sections = SECTIONS.filter(
    (section) =>
      (!section.upload || mayUploadTo(active)) &&
      (!section.admin || role === "admin"),
  );
  return (
    <nav className="section-menu" aria-label={t("settings.sections")}>
      {sections.map((section) => (
        <Link
          key={section.to}
          to={section.to}
          className="section-menu-link"
          // Account is `/settings` itself, which every other section sits
          // under: matched loosely, it would be current everywhere.
          activeOptions={{ exact: section.to === "/settings" }}
        >
          {t(section.labelKey)}
        </Link>
      ))}
    </nav>
  );
}

/**
 * The frame every section is drawn in: the menu, the section, then the
 * server's offer of its own source, which every section shares.
 */
export function SettingsHub() {
  return (
    <>
      <SettingsSections />
      <Outlet />
      <SourceOffer />
    </>
  );
}
