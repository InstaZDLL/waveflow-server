import { useQuery } from "@tanstack/react-query";
import {
  type FormEvent,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";

import {
  ApiError,
  deleteBookmark,
  deletePodcastEpisode,
  downloadPodcastEpisode,
  formatDuration,
  type PodcastChannel,
  type PodcastEpisode,
  refreshPodcasts,
  setBookmark,
  streamUrl,
  subscribePodcast,
  unsubscribePodcast,
} from "../api";
import { podcastChannelQuery, podcastsQuery } from "../api/queries";
import { Icon } from "../components/icons";
import {
  EmptyState,
  Loading,
  PageHeader,
  useReload,
} from "../components/page-shell";
import { usePlayer } from "../player/player";
import { type TranslationKey, useI18n } from "../state/i18n";

/** How often a playing episode's position is written to its bookmark. */
const SAVE_EVERY_MS = 15_000;

/**
 * The position an episode resumes from: its bookmark, unless it was left
 * within the last few seconds, which is an episode heard to the end.
 */
export function resumeSeconds(episode: {
  position_ms: number | null;
  duration_ms: number | null;
}): number {
  const position = episode.position_ms ?? 0;
  if (position <= 0) return 0;
  if (episode.duration_ms && episode.duration_ms - position < 5_000) return 0;
  return position / 1000;
}

const statusKey = (status: PodcastEpisode["status"]) =>
  `podcasts.status.${status}` as TranslationKey;

/**
 * Podcasts (RFC-025): the channels this server is subscribed to, their
 * episodes, and a player of the page's own.
 *
 * An episode is not a track: it has no album, it is not scrobbled, and the
 * queue the server keeps holds tracks only. So it plays from an element of
 * its own, as a radio station does, and the music player is paused when it
 * starts. Its position is the bookmark the server keeps for it, written while
 * it plays and read back to resume.
 */
export function PodcastsPage() {
  const { t } = useI18n();
  const player = usePlayer();
  const { data: overview, error } = useQuery(podcastsQuery());
  const reload = useReload(podcastsQuery().queryKey);
  const [url, setUrl] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  async function subscribe(event: FormEvent) {
    event.preventDefault();
    if (busy) return;
    setBusy(true);
    setMessage(null);
    try {
      const channel = await subscribePodcast(url.trim());
      setUrl("");
      setSelected(channel.id);
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

  async function unsubscribe(channel: PodcastChannel) {
    if (busy) return;
    setBusy(true);
    setMessage(null);
    try {
      await unsubscribePodcast(channel.id);
      if (selected === channel.id) setSelected(null);
      reload();
    } catch {
      setMessage(t("podcasts.mutationError"));
    } finally {
      setBusy(false);
    }
  }

  if (!overview) return <Loading error={error} />;

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
        <form
          className="inline-form"
          onSubmit={(event) => void subscribe(event)}
        >
          <input
            type="url"
            value={url}
            onChange={(event) => setUrl(event.target.value)}
            placeholder="https://example.org/feed.xml"
            aria-label={t("podcasts.url")}
            required
            disabled={busy}
          />
          <button type="submit" disabled={busy || !url.trim()}>
            {t("podcasts.subscribe")}
          </button>
        </form>
      ) : null}
      {message ? (
        <p className="muted" role="status">
          {message}
        </p>
      ) : null}
      {overview.channels.length ? (
        <ul className="resource-list podcast-channels">
          {overview.channels.map((channel) => (
            <li key={channel.id}>
              <div>
                <strong>{channel.title}</strong>
                <span className="muted">
                  {t("podcasts.episodes", { count: channel.episode_count })}
                  {channel.status === "error"
                    ? ` · ${t("podcasts.failed", { reason: channel.error ?? "" })}`
                    : ""}
                </span>
              </div>
              <button
                type="button"
                aria-pressed={selected === channel.id}
                onClick={() =>
                  setSelected(selected === channel.id ? null : channel.id)
                }
              >
                {selected === channel.id
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
      ) : (
        <EmptyState message={t("podcasts.empty")} />
      )}
      {selected ? (
        <ChannelEpisodes
          channelId={selected}
          canManage={overview.can_manage}
          pauseMusic={() => {
            if (player.playing) player.toggle();
          }}
          musicPlaying={player.playing}
        />
      ) : null}
    </section>
  );
}

function ChannelEpisodes({
  channelId,
  canManage,
  pauseMusic,
  musicPlaying,
}: {
  channelId: string;
  canManage: boolean;
  pauseMusic: () => void;
  musicPlaying: boolean;
}) {
  const { t } = useI18n();
  const { data: channel, error } = useQuery(podcastChannelQuery(channelId));
  const reload = useReload(podcastChannelQuery(channelId).queryKey);
  const audio = useRef<HTMLAudioElement | null>(null);
  const [playing, setPlaying] = useState<PodcastEpisode | null>(null);
  const playingRef = useRef<PodcastEpisode | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const lastSaved = useRef(0);

  const savePosition = useCallback(async () => {
    const episode = playingRef.current;
    const element = audio.current;
    if (!episode || !element) return;
    const position = Math.round(element.currentTime * 1000);
    lastSaved.current = Date.now();
    try {
      if (element.ended) await deleteBookmark(episode.id);
      else await setBookmark(episode.id, position);
    } catch {
      // A position that did not save is the previous one; nothing is lost
      // that the next save will not write.
    }
  }, []);

  const stop = useCallback(() => {
    const element = audio.current;
    if (element && playingRef.current) {
      element.pause();
      void savePosition().then(reload);
    }
    playingRef.current = null;
    setPlaying(null);
  }, [reload, savePosition]);

  // The music player and an episode must not sound at once.
  useEffect(() => {
    if (musicPlaying && playingRef.current) stop();
  }, [musicPlaying, stop]);

  useEffect(
    () => () => {
      // Leaving the page stops the episode, with its place kept: this page
      // owns the only control that could stop it.
      if (audio.current && playingRef.current) {
        audio.current.pause();
        void savePosition();
      }
    },
    [savePosition],
  );

  async function play(episode: PodcastEpisode) {
    setMessage(null);
    pauseMusic();
    try {
      const { url } = await streamUrl(episode.id);
      const element = audio.current ?? new Audio();
      audio.current = element;
      element.ontimeupdate = () => {
        if (Date.now() - lastSaved.current >= SAVE_EVERY_MS)
          void savePosition();
      };
      element.onpause = () => void savePosition();
      element.onended = () => {
        void savePosition().then(reload);
        playingRef.current = null;
        setPlaying(null);
      };
      element.onerror = () => {
        playingRef.current = null;
        setPlaying(null);
        setMessage(t("podcasts.playbackError"));
      };
      element.src = url;
      const resume = resumeSeconds(episode);
      if (resume > 0) {
        element.addEventListener(
          "loadedmetadata",
          () => {
            element.currentTime = resume;
          },
          { once: true },
        );
      }
      playingRef.current = episode;
      lastSaved.current = Date.now();
      setPlaying(episode);
      await element.play();
    } catch (failure) {
      if (failure instanceof DOMException && failure.name === "AbortError") {
        return;
      }
      playingRef.current = null;
      setPlaying(null);
      setMessage(t("podcasts.playbackError"));
    }
  }

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
      className="collection podcast-episodes"
      aria-labelledby="podcast-title"
    >
      <header className="collection-header">
        <div>
          <h3 id="podcast-title">{channel.title}</h3>
          {channel.description ? (
            <p className="muted">{channel.description}</p>
          ) : null}
        </div>
        {canManage ? (
          <button type="button" onClick={() => reload()}>
            {t("podcasts.reload")}
          </button>
        ) : null}
      </header>
      {message ? <p className="error">{message}</p> : null}
      {episodes.length ? (
        <ul className="resource-list">
          {episodes.map((episode) => {
            const isPlaying = playing?.id === episode.id;
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
                    onClick={() => (isPlaying ? stop() : void play(episode))}
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
                      if (isPlaying) stop();
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
