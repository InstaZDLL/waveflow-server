// What the catalogue's screens show an account that reaches no library.
import { useNavigate } from "@tanstack/react-router";
import { useEffect } from "react";

import { currentUser } from "../api";
import { EmptyState } from "../components/layout/page-shell";
import { useI18n } from "../state/i18n";
import { useLibraryScope } from "../state/library-scope";

/** How often the list is asked again while it is empty. */
export const NO_LIBRARY_POLL_MS = 5_000;

/** How many times, so an account nobody will ever share with stops asking. */
export const NO_LIBRARY_POLLS = 24;

/**
 * In place of a catalogue screen while the account reaches no library.
 *
 * Every screen of the catalogue used to answer that on its own, as "nothing
 * here yet", and none said why: a fresh server opened on empty albums, songs
 * and folders, and an upload had nowhere to go, with no word that a library
 * had to be registered first. An administrator is shown the way to the form
 * that registers one; anyone else, that the administrator holds the key.
 *
 * The list is asked again while this shows, for two minutes. A library can
 * appear without this client doing anything: setup answers after five
 * seconds whether or not the first library is made, and makes it after the
 * answer on a slow mount; an administrator may share one from elsewhere.
 * Nothing else would ask, since the library feed has nothing to follow yet.
 */
export function NoLibrary() {
  const { t } = useI18n();
  const navigate = useNavigate();
  const { refresh } = useLibraryScope();
  const admin = currentUser()?.role === "admin";
  useEffect(() => {
    let polls = 0;
    const timer = window.setInterval(() => {
      polls += 1;
      refresh();
      if (polls >= NO_LIBRARY_POLLS) window.clearInterval(timer);
    }, NO_LIBRARY_POLL_MS);
    return () => window.clearInterval(timer);
  }, [refresh]);
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
