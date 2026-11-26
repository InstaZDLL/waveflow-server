// What the catalogue's screens show an account that reaches no library.
import { useNavigate } from "@tanstack/react-router";

import { currentUser } from "../api";
import { EmptyState } from "../components/layout/page-shell";
import { useI18n } from "../state/i18n";

/**
 * In place of a catalogue screen while the account reaches no library.
 *
 * Every screen of the catalogue used to answer that on its own, as "nothing
 * here yet", and none said why: a fresh server opened on empty albums, songs
 * and folders, and an upload had nowhere to go, with no word that a library
 * had to be registered first. An administrator is shown the way to the form
 * that registers one; anyone else, that the administrator holds the key.
 */
export function NoLibrary() {
  const { t } = useI18n();
  const navigate = useNavigate();
  const admin = currentUser()?.role === "admin";
  return (
    <EmptyState
      icon="folder"
      message={t("library.none")}
      detail={admin ? t("library.noneAdmin") : t("library.noneMember")}
    >
      {admin ? (
        <button
          type="button"
          onClick={() => void navigate({ to: "/settings/admin" })}
        >
          {t("library.noneAction")}
        </button>
      ) : null}
    </EmptyState>
  );
}
