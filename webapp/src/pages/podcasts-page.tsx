import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { type FormEvent, useEffect, useRef, useState } from "react";

import {
  ApiError,
  deletePodcastEpisode,
  downloadPodcastEpisode,
  formatDuration,
  type PodcastChannel,
  type PodcastEpisode,
  refreshPodcasts,
  subscribePodcast,
  unsubscribePodcast,
} from "../api";
import { podcastChannelQuery, podcastsQuery } from "../api/queries";
import { Artwork } from "../components/artwork";
import { Icon } from "../components/icons";
import {
  EmptyState,
  Loading,
  PageHeader,
  useReload,
} from "../components/page-shell";
import { resumeSeconds } from "../lib/aside";
import { usePlayer } from "../player/player";
import { type TranslationKey, useI18n } from "../state/i18n";

const statusKey = (status: PodcastEpisode["status"]) =>
  `podcasts.status.${status}` as TranslationKey;

/**
 * From this window width the channels and the open channel's episodes sit
 * side by side; below it they stack. The same breakpoint as the
 * `.podcast-workspace` rule in `styles.css`.
 */
const TWO_PANELS = "(min-width: 64rem)";

function twoPanelsQuery(): MediaQueryList | null {
  return typeof window.matchMedia === "function"
    ? window.matchMedia(TWO_PANELS)
    : null;
}

/** Whether the window is wide enough for the two panels. */
function useTwoPanels(): boolean {
  const [wide, setWide] = useState(() => twoPanelsQuery()?.matches ?? false);
  useEffect(() => {
    const query = twoPanelsQuery();
    if (!query) return;
    const onChange = () => setWide(query.matches);
    onChange();
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);
  return wide;
}

/**
 * Podcasts (RFC-025): the channels this server is subscribed to and their
 * episodes.
 *
 * An episode is not a track: it has no album, it is not scrobbled, and the
 * queue the server keeps holds tracks only. So the player plays it aside from
 * the queue, as it does a radio station, and pauses the music under it. Its
 * position is the bookmark the server keeps for it, which the player writes
 * while it plays and this page reads back to say where it resumes.
 *
 * The open channel is `?channel=` in the address, so Back returns to the
 * channel before and a reload keeps the one being read. On a wide window the
 * episodes sit beside the channels, and the panel is never empty: with no
 * channel named, or one that is gone, it shows the first. On a narrow one the
 * episodes open under the list, and only when asked for, as they always have.
 */
export function PodcastsPage({ channel }: { channel?: string }) {
  const { t } = useI18n();
  const navigate = useNavigate();
  const wide = useTwoPanels();
  const { data: overview, error } = useQuery(podcastsQuery());
  const reload = useReload(podcastsQuery().queryKey);
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  /** The feed field, which the empty state's action hands focus to. */
  const feedField = useRef<HTMLInputElement>(null);

  async function subscribe(event: FormEvent) {
    event.preventDefault();
    if (busy) return;
    setBusy(true);
    setMessage(null);
    try {
      const created = await subscribePodcast(url.trim());
      setUrl("");
      choose(created.id);
      reload();
    } catch (failure) {
      setMessage(
        failure instanceof ApiError && failure.status === 409
          ? t("podcasts.conflict")
          : failure instanceof ApiError && failure.status === 422
            ? t("podcasts.refused")
            : t("podcasts.subscribeError"),
      );
    } finally {
      setBusy(false);
    }
  }

  async function refresh() {
    setMessage(null);
    try {
      await refreshPodcasts();
      setMessage(t("podcasts.refreshing"));
    } catch {
      setMessage(t("podcasts.mutationError"));
    }
  }

  /** Opens a channel, or closes the open one, through the address. */
  function choose(id: string | null, replace = false) {
    void navigate({
      to: "/podcasts",
      search: id ? { channel: id } : {},
      replace,
    });
  }

  async function unsubscribe(gone: PodcastChannel) {
    if (busy) return;
    setBusy(true);
    setMessage(null);
    try {
      await unsubscribePodcast(gone.id);
      // Replaced rather than pushed: Back should not lead to a channel that
      // no longer exists.
      if (channel === gone.id) choose(null, true);
      reload();
    } catch {
      setMessage(t("podcasts.mutationError"));
    } finally {
      setBusy(false);
    }
  }

  if (!overview) return <Loading error={error} />;

  const named = overview.channels.find((entry) => entry.id === channel);
  const open = named ?? (wide ? overview.channels[0] : undefined);
  const selected = open?.id ?? null;

  return (
    <section className="podcasts-page">
      <PageHeader
        kicker={t("nav.group.listen")}
        title={t("nav.podcasts")}
        detail={t("podcasts.detail")}
      >
        {overview.can_manage ? (
          <button type="button" onClick={() => void refresh()}>
            {t("podcasts.refresh")}
          </button>
        ) : null}
      </PageHeader>
      {overview.enabled ? null : <p className="muted">{t("podcasts.off")}</p>}
      {overview.can_manage ? (
        <>
          <form
            className="inline-form"
            onSubmit={(event) => void subscribe(event)}
          >
            <input
              ref={feedField}
              type="url"
              value={url}
              onChange={(event) => setUrl(event.target.value)}
              placeholder="https://example.org/feed.xml"
              aria-label={t("podcasts.url")}
              aria-describedby="podcast-shared"
              required
              disabled={busy}
            />
            <button type="submit" disabled={busy || !url.trim()}>
              {t("podcasts.subscribe")}
            </button>
          </form>
          {/* One list per instance: a private feed's episodes reach every
              account once it is added, though its address does not. */}
          <p id="podcast-shared" className="muted">
            {t("podcasts.shared")}
          </p>
        </>
      ) : null}
      {message ? (
        <p className="muted" role="status">
          {message}
        </p>
      ) : null}
      {overview.channels.length ? (
        <div className="podcast-workspace">
          <ul
            className="resource-list podcast-channels"
            aria-label={t("podcasts.channels")}
          >
            {overview.channels.map((channel) => (
              <li
                key={channel.id}
                className={selected === channel.id ? "selected" : undefined}
              >
                <div className="resource-with-cover">
                  {/* Fetched by the server into the artwork store: the feed's
                    own host is never asked by the browser. */}
                  <Artwork
                    artworkId={channel.artwork_hash}
                    title={channel.title}
                    className="resource-cover"
                    placeholder={<Icon name="podcasts" size={20} />}
                  />
                  <div className="resource-copy">
                    <strong>{channel.title}</strong>
                    <span className="muted">
                      {t("podcasts.episodes", { count: channel.episode_count })}
                      {channel.status === "error"
                        ? ` · ${t("podcasts.failed", { reason: channel.error ?? "" })}`
                        : ""}
                    </span>
                  </div>
                </div>
                {/* Beside the list, a channel is chosen rather than toggled:
                  the panel always shows one, so there is nothing to hide.
                  Focus stays on this button; the status below says what the
                  panel now holds. */}
                <button
                  type="button"
                  aria-pressed={selected === channel.id}
                  aria-controls={
                    selected === channel.id ? "podcast-panel" : undefined
                  }
                  onClick={() =>
                    wide
                      ? choose(channel.id)
                      : choose(selected === channel.id ? null : channel.id)
                  }
                >
                  {selected === channel.id && !wide
                    ? t("podcasts.hide")
                    : t("podcasts.open")}
                </button>
                {overview.can_manage ? (
                  <button
                    type="button"
                    className="danger"
                    disabled={busy}
                    onClick={() => void unsubscribe(channel)}
                  >
                    {t("podcasts.unsubscribe")}
                  </button>
                ) : null}
              </li>
            ))}
          </ul>
          {open ? (
            <ChannelEpisodes
              // Another channel is another list. The episode playing is the
              // player's, and carries on whichever list is open.
              key={open.id}
              channelId={open.id}
              canManage={overview.can_manage}
            />
          ) : null}
        </div>
      ) : (
        // Said to everyone, since a reader meets this page empty too; only a
        // manager is offered the way out, which is the field above.
        <EmptyState
          icon="podcasts"
          message={t("podcasts.empty")}
          detail={t("podcasts.emptyDetail")}
        >
          {overview.can_manage ? (
            <button
              type="button"
              className="action-primary"
              onClick={() => feedField.current?.focus()}
            >
              {t("podcasts.addFirst")}
            </button>
          ) : null}
        </EmptyState>
      )}
      {/* Said once the panel changes: a channel chosen from the list keeps
          focus on its button, so the new episodes are announced instead. */}
      <p className="visually-hidden" role="status">
        {open ? t("podcasts.showing", { title: open.title }) : ""}
      </p>
    </section>
  );
}

function ChannelEpisodes({
  channelId,
  canManage,
}: {
  channelId: string;
  canManage: boolean;
}) {
  const { t } = useI18n();
  const player = usePlayer();
  const { data: channel, error } = useQuery(podcastChannelQuery(channelId));
  const reload = useReload(podcastChannelQuery(channelId).queryKey);
  const [message, setMessage] = useState<string | null>(null);
  // The episode in the bar, if it is one; whether it plays is the player's.
  const current =
    player.aside?.kind === "episode" ? player.aside.episode : null;

  async function act(action: () => Promise<void>) {
    setMessage(null);
    try {
      await action();
      reload();
    } catch {
      setMessage(t("podcasts.mutationError"));
    }
  }

  if (!channel) return <Loading error={error} />;
  const episodes = channel.episodes ?? [];

  return (
    <section
      id="podcast-panel"
      className="collection podcast-episodes"
      aria-labelledby="podcast-title"
    >
      <header className="collection-header">
        <div className="resource-with-cover podcast-heading">
          <Artwork
            artworkId={channel.artwork_hash}
            title={channel.title}
            className="podcast-cover"
            placeholder={<Icon name="podcasts" size={32} />}
          />
          <div className="resource-copy">
            <h3 id="podcast-title">{channel.title}</h3>
            {channel.description ? (
              <p className="muted">{channel.description}</p>
            ) : null}
          </div>
        </div>
        {canManage ? (
          <button type="button" onClick={() => reload()}>
            {t("podcasts.reload")}
          </button>
        ) : null}
      </header>
      {message ? <p className="error">{message}</p> : null}
      {current?.channel_id === channelId && player.asideError ? (
        <p className="error">{t("podcasts.playbackError")}</p>
      ) : null}
      {episodes.length ? (
        <ul className="resource-list">
          {episodes.map((episode) => {
            const isPlaying = current?.id === episode.id && player.asidePlaying;
            const resume = resumeSeconds(episode);
            return (
              <li key={episode.id}>
                <div>
                  <strong>{episode.title}</strong>
                  <span className="muted">
                    {episode.published_at
                      ? new Date(episode.published_at).toLocaleDateString()
                      : ""}
                    {episode.duration_ms
                      ? ` · ${formatDuration(episode.duration_ms)}`
                      : ""}
                    {` · ${t(statusKey(episode.status))}`}
                    {episode.status === "error" && episode.error
                      ? ` (${episode.error})`
                      : ""}
                    {resume > 0
                      ? ` · ${t("podcasts.resumeAt", { time: formatDuration(resume * 1000) })}`
                      : ""}
                  </span>
                </div>
                {episode.status === "completed" ? (
                  <button
                    type="button"
                    aria-label={
                      isPlaying
                        ? t("podcasts.pause", { title: episode.title })
                        : t("podcasts.play", { title: episode.title })
                    }
                    onClick={() =>
                      isPlaying
                        ? player.toggleAside()
                        : player.playEpisode(episode)
                    }
                  >
                    <Icon name={isPlaying ? "pause" : "play"} />
                  </button>
                ) : canManage && episode.status !== "downloading" ? (
                  <button
                    type="button"
                    onClick={() =>
                      void act(() => downloadPodcastEpisode(episode.id))
                    }
                  >
                    {t("podcasts.download")}
                  </button>
                ) : (
                  <span />
                )}
                {canManage && episode.status === "completed" ? (
                  <button
                    type="button"
                    className="danger"
                    onClick={() => {
                      // Out of the bar, playing or paused: its file is going.
                      player.stopAside(episode.id);
                      void act(() => deletePodcastEpisode(episode.id));
                    }}
                  >
                    {t("common.delete")}
                  </button>
                ) : null}
              </li>
            );
          })}
        </ul>
      ) : (
        <EmptyState message={t("podcasts.noEpisodes")} />
      )}
    </section>
  );
}
