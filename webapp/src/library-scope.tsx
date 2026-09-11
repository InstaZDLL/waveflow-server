import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";

import { type Library, listLibraries } from "./api";
import { useI18n } from "./i18n";

const STORAGE_KEY = "waveflow.library";

/**
 * The one library the web client is looking at.
 *
 * The contract, decided on 2026-09-07: *the web UI always operates within one
 * active library; changing library changes the catalogue's scope, it does not
 * merge catalogues.* Aggregating would have to answer what a record held by two
 * libraries is — one album twice, one album with two provenances, or two
 * editions — and which of them supplies the cover, the year, the tags. That is
 * the identity problem `src/pid.rs` settles for files, asked again a layer up.
 *
 * The desktop client may unify its sources; the server's own interface is the
 * view of one server catalogue and does not have to reproduce the client's
 * model of presentation.
 */
type LibraryScope = {
  libraries: Library[];
  /** `null` until the list has loaded, or when the account has none. */
  active: Library | null;
  setActive: (id: string) => void;
  /**
   * Whether the scope is known. Catalogue screens must wait for it: asking
   * before it settles sends one unscoped request — answering with every
   * library's albums, which is the thing this scope exists to prevent — and
   * then a second, scoped one that replaces it.
   */
  ready: boolean;
};

const LibraryScopeContext = createContext<LibraryScope | null>(null);

/** Read the last selected library without requiring storage to be available. */
function readStored(): string | null {
  try {
    return localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

/** Load and provide the account's active library selection. */
export function LibraryScopeProvider({ children }: { children: ReactNode }) {
  const [libraries, setLibraries] = useState<Library[]>([]);
  const [activeId, setActiveId] = useState<string | null>(() => readStored());
  const [ready, setReady] = useState(false);

  useEffect(() => {
    let cancelled = false;
    listLibraries().then(
      (found) => {
        if (cancelled) return;
        setLibraries(found);
        setReady(true);
        // A remembered library that is gone — deleted, or a membership
        // withdrawn — must not leave the client scoped to nothing.
        setActiveId((current) =>
          current && found.some((library) => library.id === current)
            ? current
            : (found[0]?.id ?? null),
        );
      },
      () => {
        // The catalogue screens would otherwise wait forever on a scope that
        // is never coming. Unscoped is the honest fallback when the list of
        // libraries cannot be had at all.
        if (!cancelled) setReady(true);
      },
    );
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!activeId) return;
    try {
      localStorage.setItem(STORAGE_KEY, activeId);
    } catch {
      // The scope still holds for this session without persistence.
    }
  }, [activeId]);

  const value = useMemo<LibraryScope>(
    () => ({
      libraries,
      active: libraries.find((library) => library.id === activeId) ?? null,
      setActive: setActiveId,
      ready,
    }),
    [libraries, activeId, ready],
  );

  return (
    <LibraryScopeContext.Provider value={value}>
      {children}
    </LibraryScopeContext.Provider>
  );
}

/** Access the active library and the libraries available to the account. */
export function useLibraryScope(): LibraryScope {
  const value = useContext(LibraryScopeContext);
  if (!value) throw new Error("useLibraryScope requires LibraryScopeProvider");
  return value;
}

/**
 * Whether the account may correct track tags in a library: its owner or a
 * manager, the pair `may_write_metadata` names on the server. The server decides
 * regardless; this only keeps a screen from offering a refusal.
 */
export function mayCorrectTracks(library: Library | undefined): boolean {
  return library?.role === "owner" || library?.role === "manager";
}

/** The active library's id, or `undefined` to mean "do not scope the call". */
export function useScopeId(): string | undefined {
  return useLibraryScope().active?.id;
}

/** Render the active-library selector when the account has multiple choices. */
export function LibraryPicker() {
  const { libraries, active, setActive } = useLibraryScope();
  const { t } = useI18n();
  const onChange = useCallback((id: string) => setActive(id), [setActive]);
  // One library is not a choice, and a select with a single option is a
  // control that does nothing. Nothing at all is shown until the list loads,
  // so the sidebar does not flash a picker that then disappears.
  if (libraries.length < 2) return null;
  return (
    <label className="library-picker">
      <span>{t("library.active")}</span>
      <select
        aria-label={t("library.active")}
        value={active?.id ?? ""}
        onChange={(event) => onChange(event.target.value)}
      >
        {libraries.map((library) => (
          <option key={library.id} value={library.id}>
            {library.name}
          </option>
        ))}
      </select>
    </label>
  );
}
