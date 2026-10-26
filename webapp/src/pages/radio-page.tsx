import { useQuery } from "@tanstack/react-query";
import {
  type FormEvent,
  type ReactNode,
  useEffect,
  useId,
  useRef,
  useState,
} from "react";

import {
  ApiError,
  createRadioStation,
  deleteRadioStation,
  type RadioStation,
  replaceRadioStation,
} from "../api";
import { radioStationsQuery } from "../api/queries";
import { ActionMenu } from "../components/action-menu";
import { Artwork } from "../components/artwork";
import { Icon } from "../components/icons";
import {
  EmptyState,
  Loading,
  PageHeader,
  useReload,
} from "../components/page-shell";
import { usePlayer } from "../player/player";
import { useI18n } from "../state/i18n";

/**
 * The same rule the server applies, applied early enough to say so.
 *
 * Absolute, `http` or `https`. The point is not to protect the server — it
 * validates for itself, and it has to, since the facade accepts the same
 * writes — but to tell somebody typing why the button is refusing them instead
 * of making them send a request to find out.
 */
export function isPlayableAddress(value: string): boolean {
  const trimmed = value.trim();
  if (!trimmed) return false;
  try {
    const url = new URL(trimmed);
    return url.protocol === "http:" || url.protocol === "https:";
  } catch {
    return false;
  }
}

/**
 * Whether the browser will refuse this station outright.
 *
 * A clear-text stream on a page served over TLS is mixed content, and every
 * current browser blocks it in the audio element without asking. The server
 * stores such an address quite happily — it never fetches it, so nothing there
 * can tell — and the failure then arrives as a stream that simply does not
 * play. Saying it in advance is the only place this can be said usefully.
 */
export function isBlockedAsMixedContent(
  streamUrl: string,
  pageProtocol: string,
): boolean {
  return pageProtocol === "https:" && streamUrl.trim().startsWith("http:");
}

type Draft = {
  name: string;
  stream_url: string;
  homepage_url: string;
  image_url: string;
};

const emptyDraft: Draft = {
  name: "",
  stream_url: "",
  homepage_url: "",
  image_url: "",
};

/**
 * The account's stations, as a grid of cards: each one's image, its name and
 * the button that plays it. Adding and editing happen in a dialog that holds
 * the form; editing and deleting are reached through the card's "⋯" menu,
 * where a delete is asked to be confirmed first.
 */
export function RadioPage() {
  const { t } = useI18n();
  const player = usePlayer();
  const { data: stations, error } = useQuery(radioStationsQuery());
  const reload = useReload(radioStationsQuery().queryKey);
  const [draft, setDraft] = useState<Draft>(emptyDraft);
  /** Whether the dialog holding the form is open. */
  const [formOpen, setFormOpen] = useState(false);
  /** The station the form is replacing, or `null` when it is adding one. */
  const [editing, setEditing] = useState<RadioStation | null>(null);
  const [mutationError, setMutationError] = useState<string | null>(null);
  /** What had focus when the dialog opened, given focus back when it closes. */
  const returnTo = useRef<HTMLElement | null>(null);
  // The station is the player's, not this page's: it plays in the bar, and
  // carries on when the reader goes elsewhere. What this page shows of it is
  // read from there.
  const live = player.aside?.kind === "live" ? player.aside.station : null;
  const playingId = live && player.asidePlaying ? live.id : null;
  /**
   * True while a write is in flight, which closes the form and the two
   * per-station writes.
   *
   * **Not polish.** A second click on `Add station` before the first answered
   * created a second station — the server has no reason to refuse one — and
   * starting to edit another station mid-save made the save clear a form the
   * reader had just filled, because `editing` and `draft` are read again
   * after the `await`. Closing the form removes both, where comparing the
   * values afterwards would have removed only the second (constats de revue,
   * pris). The dialog cannot be dismissed while it saves, for the same reason.
   * Listening stays available throughout: it writes nothing, and a reader
   * waiting on a delete has no reason to lose the station they are hearing.
   */
  const [busy, setBusy] = useState(false);

  // Focus goes back once the dialog has left the page: while it is still
  // open and modal, everything behind it is inert and refuses focus.
  const wasOpen = useRef(false);
  useEffect(() => {
    if (formOpen) {
      wasOpen.current = true;
    } else if (wasOpen.current) {
      wasOpen.current = false;
      returnTo.current?.focus();
    }
  }, [formOpen]);

  if (!stations) return <Loading error={error} />;

  function openForm(station: RadioStation | null) {
    returnTo.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    setMutationError(null);
    setEditing(station);
    setDraft(
      station
        ? {
            name: station.name,
            stream_url: station.stream_url,
            homepage_url: station.homepage_url ?? "",
            image_url: station.image_url ?? "",
          }
        : emptyDraft,
    );
    setFormOpen(true);
  }

  function closeForm() {
    setFormOpen(false);
    setEditing(null);
    setDraft(emptyDraft);
    setMutationError(null);
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (busy) return;
    setBusy(true);
    setMutationError(null);
    // Sent whole, every time: the server replaces the station rather than
    // patching it, so an empty homepage field clears the stored one.
    const payload = {
      name: draft.name,
      stream_url: draft.stream_url,
      homepage_url: draft.homepage_url.trim() ? draft.homepage_url : null,
      // Sent every time as well, so emptying the field clears the image. The
      // server fetches it once, when it is new; the same address saved again
      // is not fetched twice, and one whose fetch failed is tried again.
      image_url: draft.image_url.trim() ? draft.image_url : null,
    };
    try {
      if (editing) {
        await replaceRadioStation(editing.id, payload);
      } else {
        await createRadioStation(payload);
      }
      // Safe to clear because the form was closed for the whole round trip:
      // nothing the reader did in the meantime is being thrown away.
      closeForm();
      reload();
    } catch (failure) {
      const full = failure instanceof ApiError && failure.status === 409;
      setMutationError(full ? t("radio.full") : t("radio.saveError"));
    } finally {
      setBusy(false);
    }
  }

  async function remove(station: RadioStation) {
    if (busy) return;
    if (!window.confirm(t("radio.confirmDelete", { name: station.name })))
      return;
    setBusy(true);
    setMutationError(null);
    try {
      await deleteRadioStation(station.id);
      // Asked of the player by id, and not decided here from `live`: the
      // station playing now is not necessarily the one that was playing when
      // this request left, and stopping on that stale value would either leave
      // a deleted station sounding or stop one the reader had just started.
      player.stopAside(station.id);
      reload();
    } catch {
      setMutationError(t("radio.deleteError"));
    } finally {
      setBusy(false);
    }
  }

  const addressRefused = draft.stream_url.trim()
    ? !isPlayableAddress(draft.stream_url)
    : false;
  const homepageRefused = draft.homepage_url.trim()
    ? !isPlayableAddress(draft.homepage_url)
    : false;
  const imageRefused = draft.image_url.trim()
    ? !isPlayableAddress(draft.image_url)
    : false;
  const submittable =
    draft.name.trim().length > 0 &&
    !addressRefused &&
    !homepageRefused &&
    !imageRefused;

  return (
    <section className="radio-page">
      <PageHeader
        kicker={t("nav.group.listen")}
        title={t("nav.radio")}
        detail={t("radio.detail")}
      >
        <button
          type="button"
          className="action-primary"
          disabled={busy}
          onClick={() => openForm(null)}
        >
          {t("radio.add")}
        </button>
      </PageHeader>
      {mutationError && !formOpen ? (
        <p className="error" role="alert">
          {mutationError}
        </p>
      ) : null}
      {live && player.asideError ? (
        <p className="error">{t("radio.playbackError")}</p>
      ) : null}
      {stations.length ? (
        <ul className="station-grid" aria-label={t("radio.stations")}>
          {stations.map((station) => {
            const playing = playingId === station.id;
            return (
              <li key={station.id} className="station-card">
                <div className="station-art">
                  <Artwork
                    artworkId={station.artwork_hash}
                    title={station.name}
                    className="station-cover"
                    placeholder={<Icon name="radio" size={36} />}
                  />
                  <button
                    type="button"
                    className="station-play"
                    aria-pressed={playing}
                    onClick={() =>
                      playing ? player.toggleAside() : player.playLive(station)
                    }
                    aria-label={
                      playing
                        ? t("radio.stopNamed", { name: station.name })
                        : t("radio.listenTo", { name: station.name })
                    }
                  >
                    <Icon name={playing ? "stop" : "play"} size={20} />
                  </button>
                </div>
                <div className="station-copy">
                  <div className="station-title">
                    <strong className="station-name">{station.name}</strong>
                    <ActionMenu
                      label={t("radio.actions", { name: station.name })}
                      disabled={busy}
                      items={[
                        {
                          key: "edit",
                          label: t("radio.edit"),
                          run: () => openForm(station),
                        },
                        {
                          key: "delete",
                          label: t("common.delete"),
                          danger: true,
                          run: () => void remove(station),
                        },
                      ]}
                    />
                  </div>
                  <span
                    className="muted station-url"
                    title={station.stream_url}
                  >
                    {station.stream_url}
                  </span>
                  {isBlockedAsMixedContent(
                    station.stream_url,
                    window.location.protocol,
                  ) ? (
                    <span className="muted station-note">
                      {t("radio.mixedContent")}
                    </span>
                  ) : null}
                  {station.image_url && !station.artwork_hash ? (
                    <span className="muted station-note">
                      {t("radio.imageMissing")}
                    </span>
                  ) : null}
                  {station.homepage_url ? (
                    <a
                      className="station-link"
                      href={station.homepage_url}
                      target="_blank"
                      rel="noreferrer noopener"
                    >
                      {t("radio.visit")}
                    </a>
                  ) : null}
                </div>
              </li>
            );
          })}
        </ul>
      ) : (
        <EmptyState icon="radio" message={t("radio.empty")} />
      )}
      {formOpen ? (
        <StationDialog
          title={
            editing
              ? t("radio.editTitle", { name: editing.name })
              : t("radio.newTitle")
          }
          locked={busy}
          onClose={closeForm}
        >
          {(close) => (
            <form
              className="stacked-form radio-form"
              onSubmit={(event) => void submit(event)}
            >
              <label>
                {t("radio.name")}
                <input
                  value={draft.name}
                  onChange={(event) =>
                    setDraft({ ...draft, name: event.target.value })
                  }
                  required
                  disabled={busy}
                />
              </label>
              <label>
                {t("radio.address")}
                <input
                  type="url"
                  value={draft.stream_url}
                  onChange={(event) =>
                    setDraft({ ...draft, stream_url: event.target.value })
                  }
                  placeholder="https://radio.example.org/stream"
                  required
                  disabled={busy}
                />
              </label>
              <label>
                {t("radio.homepage")}
                <input
                  type="url"
                  value={draft.homepage_url}
                  onChange={(event) =>
                    setDraft({ ...draft, homepage_url: event.target.value })
                  }
                  disabled={busy}
                />
              </label>
              <label>
                {t("radio.image")}
                <input
                  type="url"
                  value={draft.image_url}
                  onChange={(event) =>
                    setDraft({ ...draft, image_url: event.target.value })
                  }
                  placeholder="https://radio.example.org/logo.png"
                  aria-describedby="radio-image-hint"
                  disabled={busy}
                />
              </label>
              <p className="muted" id="radio-image-hint">
                {t("radio.imageHint")}
              </p>
              {addressRefused || homepageRefused || imageRefused ? (
                <p className="error">{t("radio.addressRefused")}</p>
              ) : null}
              {isBlockedAsMixedContent(
                draft.stream_url,
                window.location.protocol,
              ) ? (
                <p className="muted">{t("radio.mixedContent")}</p>
              ) : null}
              {editing ? <p className="muted">{t("radio.replacing")}</p> : null}
              {mutationError ? (
                <p className="error" role="alert">
                  {mutationError}
                </p>
              ) : null}
              <div className="actions">
                <button type="submit" disabled={!submittable || busy}>
                  {t("radio.save")}
                </button>
                <button type="button" disabled={busy} onClick={close}>
                  {t("common.cancel")}
                </button>
              </div>
            </form>
          )}
        </StationDialog>
      ) : null}
    </section>
  );
}

/**
 * The dialog the station form sits in: a native `<dialog>` opened with
 * `showModal`, as the folder picker is, so the focus trap, Escape and the
 * inert page behind come from the platform.
 *
 * `locked` holds it open while a save is in flight: Escape, the backdrop and
 * the close button would otherwise put the form away under a request that is
 * about to report into it.
 */
function StationDialog({
  title,
  locked,
  onClose,
  children,
}: {
  title: string;
  locked: boolean;
  onClose: () => void;
  /** Handed the way to close, for the form's own Cancel button. */
  children: (close: () => void) => ReactNode;
}) {
  const { t } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();

  useEffect(() => {
    const node = dialog.current;
    if (node && !node.open) node.showModal();
  }, []);

  const close = () => {
    if (!locked) dialog.current?.close();
  };

  return (
    // biome-ignore lint/a11y/useKeyWithClickEvents: Escape closes a <dialog> natively
    <dialog
      ref={dialog}
      className="station-dialog"
      aria-labelledby={titleId}
      onClose={onClose}
      onCancel={(event) => {
        if (locked) event.preventDefault();
      }}
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
        if (!inside) close();
      }}
    >
      <header>
        <h2 id={titleId}>{title}</h2>
        <button
          type="button"
          className="properties-close"
          aria-label={t("properties.close")}
          disabled={locked}
          onClick={close}
        >
          <Icon name="close" />
        </button>
      </header>
      {children(close)}
    </dialog>
  );
}
