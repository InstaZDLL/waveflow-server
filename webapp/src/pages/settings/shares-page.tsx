import { useQuery } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";

import { ApiError, createShare, deleteShare, type Share } from "../../api";
import { sharesQuery } from "../../api/queries";
import { MAX_SHARE_TRACKS } from "../../api/shares";
import {
  EmptyState,
  Loading,
  PageHeader,
  useReload,
} from "../../components/layout/page-shell";
import { usePlayer } from "../../player/player";
import { useI18n } from "../../state/i18n";

export function SharesPage() {
  const player = usePlayer();
  const { t } = useI18n();
  const [description, setDescription] = useState("");
  // Kept as typed: an empty field is "no cap", and the number input already
  // refuses anything that is not a whole number of at least one.
  const [maxVisits, setMaxVisits] = useState("");
  const [createdShare, setCreatedShare] = useState<Share | null>(null);
  const reload = useReload(sharesQuery().queryKey);
  const [mutationError, setMutationError] = useState<string | null>(null);
  // Inside Home Assistant with no public address (RFC-030 decision 8), the
  // server refuses to make a link nobody outside the panel could open; the
  // form then gives way to why. Not rendered rather than `hidden`: the form's
  // own `display` outranks the attribute, and it stayed on screen.
  const [needsAddress, setNeedsAddress] = useState(false);
  const { data: value, error } = useQuery(sharesQuery());
  const createdShareNotice = createdShare?.url ? (
    <p>
      {t("shares.once")}{" "}
      <a className="resource-url" href={createdShare.url}>
        {createdShare.url}
      </a>
    </p>
  ) : null;
  if (!value) {
    return (
      <>
        {createdShareNotice}
        <Loading error={error} />
      </>
    );
  }

  // A share holds what one opening can hand a pass to each of; a longer
  // queue is refused by the server, so it is not sent.
  const tooLong = player.queue.length > MAX_SHARE_TRACKS;

  async function shareQueue(event: FormEvent) {
    event.preventDefault();
    if (tooLong) return;
    setMutationError(null);
    try {
      const share = await createShare(
        player.queue.map((song) => song.id),
        description,
        maxVisits === "" ? null : Number(maxVisits),
      );
      setCreatedShare(share);
      setDescription("");
      setMaxVisits("");
      reload();
    } catch (failure) {
      if (
        failure instanceof ApiError &&
        failure.code === "share_address_missing"
      ) {
        setNeedsAddress(true);
      } else {
        setMutationError(t("shares.createError"));
      }
    }
  }

  async function removeShare(id: string) {
    setMutationError(null);
    try {
      await deleteShare(id);
      setCreatedShare((current) => (current?.id === id ? null : current));
      reload();
    } catch {
      setMutationError(t("shares.deleteError"));
    }
  }

  return (
    <section>
      <PageHeader
        kicker={t("nav.settings")}
        title={t("nav.shares")}
        detail={t("shares.detail")}
      />
      {needsAddress ? (
        <p className="muted" role="status">
          {t("shares.needsAddress")}
        </p>
      ) : null}
      {needsAddress ? null : (
        <form
          className="inline-form"
          onSubmit={(event) => void shareQueue(event)}
        >
          <input
            value={description}
            onChange={(event) => setDescription(event.target.value)}
            placeholder={t("shares.description")}
            aria-label={t("shares.description")}
            required
          />
          <input
            type="number"
            inputMode="numeric"
            min={1}
            step={1}
            value={maxVisits}
            onChange={(event) => setMaxVisits(event.target.value)}
            placeholder={t("shares.maxVisits")}
            aria-label={t("shares.maxVisits")}
          />
          <button type="submit" disabled={!player.queue.length || tooLong}>
            {t("shares.create")}
          </button>
        </form>
      )}
      {!player.queue.length && !needsAddress ? (
        <p className="muted">{t("shares.needQueue")}</p>
      ) : null}
      {tooLong ? (
        <p className="muted">
          {t("shares.tooLong", {
            count: player.queue.length,
            limit: MAX_SHARE_TRACKS,
          })}
        </p>
      ) : null}
      {mutationError ? <p className="error">{mutationError}</p> : null}
      {createdShareNotice}
      {value.length ? (
        <ul className="resource-list">
          {value.map((share) => (
            <li key={share.id}>
              <div>
                <strong>{share.description ?? t("shares.defaultName")}</strong>
                {share.url ? (
                  <a className="muted resource-url" href={share.url}>
                    {share.url}
                  </a>
                ) : null}
              </div>
              <span className="muted">
                {t("common.tracks", { count: share.track_ids.length })}
                {" · "}
                {share.max_visits === null
                  ? t("shares.visits", { count: share.visit_count })
                  : t("shares.visitsOf", {
                      count: share.visit_count,
                      max: share.max_visits,
                    })}
              </span>
              <button
                type="button"
                className="danger"
                onClick={() => void removeShare(share.id)}
              >
                {t("common.delete")}
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <EmptyState message={t("shares.empty")} />
      )}
    </section>
  );
}
