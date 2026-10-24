import { type FormEvent, useEffect, useId, useRef, useState } from "react";

import {
  ApiError,
  type ArtistDetail,
  type ArtistImageCandidate,
  getArtistImageCandidates,
  pickDeezerPortrait,
} from "../api";
import { type TranslationKey, useI18n } from "../state/i18n";
import { Artwork } from "./artwork";
import { Icon } from "./icons";

/**
 * Choose an artist's portrait from Deezer (RFC-013 decision 4): the desktop's
 * "Search Deezer" tab.
 *
 * The server does the looking. It asks Deezer, fetches each candidate's
 * picture into its own store, and answers previews it holds, so this page
 * never loads anything from Deezer and Deezer never learns who is looking.
 * A pick sends a Deezer id and nothing else; the server checks it against the
 * search it made.
 *
 * A native `<dialog>` opened with `showModal`, as the folder picker is: the
 * focus trap, Escape and the inert page behind come from the platform. The
 * query starts as the artist's name and is searched at once.
 */
export function DeezerPortraitPicker({
  artist,
  onPicked,
  onClose,
}: {
  artist: ArtistDetail;
  onPicked: (next: ArtistDetail) => Promise<void>;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const [query, setQuery] = useState(artist.name);
  const [candidates, setCandidates] = useState<ArtistImageCandidate[] | null>(
    null,
  );
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<TranslationKey | null>(null);

  async function search(asked: string) {
    if (!asked.trim()) return;
    setBusy(true);
    setProblem(null);
    try {
      setCandidates(await getArtistImageCandidates(artist.id, asked));
    } catch (error) {
      setCandidates(null);
      setProblem(
        error instanceof ApiError && error.status === 404
          ? "artistImage.deezerUnavailable"
          : error instanceof ApiError && error.status === 429
            ? "artistImage.deezerBusy"
            : "artistImage.failed",
      );
    } finally {
      setBusy(false);
    }
  }

  async function pick(candidate: ArtistImageCandidate) {
    setBusy(true);
    setProblem(null);
    try {
      await onPicked(await pickDeezerPortrait(artist.id, candidate.deezer_id));
      dialog.current?.close();
    } catch (error) {
      setProblem(
        error instanceof ApiError && error.status === 422
          ? "artistImage.deezerExpired"
          : error instanceof ApiError && error.status === 404
            ? "artistImage.deezerUnavailable"
            : "artistImage.failed",
      );
    } finally {
      setBusy(false);
    }
  }

  // Searched once, as it opens: the artist's own name is the likely query.
  // The ref keeps a development double mount from asking twice, which the
  // server would answer with a 429 for the second.
  const opened = useRef(false);
  // biome-ignore lint/correctness/useExhaustiveDependencies: once, on open
  useEffect(() => {
    const node = dialog.current;
    if (node && !node.open) node.showModal();
    if (opened.current) return;
    opened.current = true;
    void search(artist.name);
  }, []);

  function submit(event: FormEvent) {
    event.preventDefault();
    void search(query);
  }

  return (
    // biome-ignore lint/a11y/useKeyWithClickEvents: Escape closes a <dialog> natively
    <dialog
      ref={dialog}
      className="folder-picker deezer-picker"
      aria-labelledby={titleId}
      aria-busy={busy}
      onClose={onClose}
      onClick={(event) => {
        // A click on the backdrop lands on the dialog itself, outside its box.
        const node = dialog.current;
        if (!node || event.target !== node) return;
        const box = node.getBoundingClientRect();
        const inside =
          event.clientX >= box.left &&
          event.clientX <= box.right &&
          event.clientY >= box.top &&
          event.clientY <= box.bottom;
        if (!inside) node.close();
      }}
    >
      <header>
        <h2 id={titleId}>{t("artistImage.deezerTitle")}</h2>
        <button
          type="button"
          className="properties-close"
          aria-label={t("properties.close")}
          onClick={() => dialog.current?.close()}
        >
          <Icon name="close" />
        </button>
      </header>
      <form className="deezer-search" onSubmit={submit}>
        <input
          type="search"
          className="deezer-search-input"
          value={query}
          maxLength={200}
          aria-label={t("artistImage.deezerQuery")}
          onChange={(event) => setQuery(event.target.value)}
        />
        <button
          type="submit"
          className="deezer-search-button"
          disabled={busy || !query.trim()}
        >
          <Icon name="search" size={16} />
          {t("artistImage.deezerSearch")}
        </button>
      </form>
      {busy && candidates === null ? (
        <p className="muted" role="status">
          {t("artistImage.deezerSearching")}
        </p>
      ) : null}
      {candidates?.length === 0 ? (
        <p className="muted" role="status">
          {t("artistImage.deezerNone")}
        </p>
      ) : null}
      {candidates?.length ? (
        <ul
          className="deezer-candidates"
          aria-label={t("artistImage.deezerResults")}
        >
          {candidates.map((candidate) => (
            <li key={candidate.deezer_id}>
              <button
                type="button"
                className="deezer-candidate"
                disabled={busy}
                onClick={() => void pick(candidate)}
              >
                <Artwork
                  artworkId={candidate.artwork_hash}
                  title={candidate.name}
                  className="deezer-candidate-preview"
                />
                <span className="deezer-candidate-name">{candidate.name}</span>
                {candidate.fans !== undefined ? (
                  <small className="muted">
                    {t("artistImage.deezerFans", { count: candidate.fans })}
                  </small>
                ) : null}
              </button>
            </li>
          ))}
        </ul>
      ) : null}
      {problem ? (
        <p className="error" role="alert">
          {t(problem)}
        </p>
      ) : null}
    </dialog>
  );
}
