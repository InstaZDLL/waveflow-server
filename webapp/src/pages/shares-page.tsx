import { useQuery } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";

import { createShare, deleteShare, type Share } from "../api";
import { sharesQuery } from "../api/queries";
import { MAX_SHARE_TRACKS } from "../api/shares";
import {
  EmptyState,
  Loading,
  PageHeader,
  useReload,
} from "../components/page-shell";
import { usePlayer } from "../player/player";
import { useI18n } from "../state/i18n";

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
    } catch {
      setMutationError(t("shares.createError"));
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
      {!player.queue.length ? (
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
