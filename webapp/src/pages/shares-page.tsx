import { useQuery } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";

import { createShare, deleteShare, type Share } from "../api";
import { sharesQuery } from "../api/queries";
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

  async function shareQueue(event: FormEvent) {
    event.preventDefault();
    setMutationError(null);
    try {
      const share = await createShare(
        player.queue.map((song) => song.id),
        description,
      );
      setCreatedShare(share);
      setDescription("");
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
        kicker={t("nav.group.account")}
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
        <button type="submit" disabled={!player.queue.length}>
          {t("shares.create")}
        </button>
      </form>
      {!player.queue.length ? (
        <p className="muted">{t("shares.needQueue")}</p>
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
