// The bar at the foot of the shell: the queue's transport, or the aside's.
import { Link } from "@tanstack/react-router";
import { type CSSProperties, useState } from "react";

import { formatDuration } from "../api";
import { Artwork } from "../components/artwork";
import { Icon } from "../components/icons";
import { type Aside, asideArtwork, asideTransport } from "../lib/aside";
import { useI18n } from "../state/i18n";
import { useAsideProgress, usePlayer, usePlayerProgress } from "./context";

/**
 * The line under the transport: where playback is, and a range to move it.
 * Shared by the queue and an episode, each with its own clock.
 */
function Scrubber({
  position,
  duration,
  onSeek,
}: {
  position: number;
  duration: number;
  onSeek: (seconds: number) => void;
}) {
  const { t } = useI18n();
  const [scrubbing, setScrubbing] = useState<number | null>(null);
  const commit = (value: number) => {
    onSeek(value);
    setScrubbing(null);
  };
  return (
    // `--played` is the share already heard. The dark themes draw that part
    // of the line as the mark's wave and leave the rest straight, beneath a
    // range input that keeps every interaction.
    <div
      className="player-progress"
      style={
        {
          "--played":
            duration > 0 ? Math.min(1, (scrubbing ?? position) / duration) : 0,
        } as CSSProperties
      }
    >
      <span>{formatDuration((scrubbing ?? position) * 1000)}</span>
      <span className="progress-wave" aria-hidden="true" />
      <input
        type="range"
        min={0}
        max={duration || 0}
        step={0.5}
        value={scrubbing ?? position}
        onChange={(event) => setScrubbing(Number(event.target.value))}
        onMouseUp={(event) => commit(Number(event.currentTarget.value))}
        onTouchEnd={(event) => commit(Number(event.currentTarget.value))}
        onKeyUp={(event) => commit(Number(event.currentTarget.value))}
        aria-label={t("player.seek")}
      />
      <span>{formatDuration(duration * 1000)}</span>
    </div>
  );
}

function QueueScrubber() {
  const player = usePlayer();
  const progress = usePlayerProgress();
  return (
    <Scrubber
      position={progress.position}
      duration={progress.duration}
      onSeek={player.seek}
    />
  );
}

function AsideScrubber() {
  const player = usePlayer();
  const progress = useAsideProgress();
  return (
    <Scrubber
      position={progress.position}
      duration={progress.duration}
      onSeek={player.seekAside}
    />
  );
}

/** The level, which is the same for the queue and for an aside. */
function LevelControls() {
  const player = usePlayer();
  const { t } = useI18n();
  return (
    <>
      <button
        type="button"
        onClick={player.toggleMute}
        aria-label={player.muted ? t("player.unmute") : t("player.mute")}
        aria-pressed={player.muted}
      >
        <Icon name={player.muted ? "muted" : "volume"} size={18} />
      </button>
      <input
        type="range"
        className="volume"
        min={0}
        max={1}
        step={0.01}
        value={player.muted ? 0 : player.volume}
        onChange={(event) => player.setVolume(Number(event.target.value))}
        aria-label={t("player.volume")}
      />
      <Link className="nav-action" to="/queue">
        <Icon name="queue" size={18} />
        <span>{t("nav.queue")}</span>
      </Link>
    </>
  );
}

/**
 * The bar while a station or an episode plays aside from the queue.
 *
 * A station is live: a badge says so, and there is nothing to seek in and no
 * next to go to. An episode has its position and a scrubber, and no next
 * either, because an episode that ends stops. Shuffle and repeat are the
 * queue's and are not offered. The second button takes the aside out of the
 * bar, which leaves the queue's track there, paused where it was.
 */
function AsideBar({ aside }: { aside: Aside }) {
  const player = usePlayer();
  const { t } = useI18n();
  const live = aside.kind === "live";
  const transport = asideTransport(aside.kind);
  const primaryLabel = player.asidePlaying
    ? live
      ? t("player.stopLive")
      : t("player.pause")
    : t("player.play");
  return (
    <footer className="player player-with-aside" data-aside={aside.kind}>
      {/* The station's image or the channel's cover, which the server
          fetched into the artwork store; the generic symbol until it comes,
          and for good when there is none. */}
      <Artwork
        artworkId={asideArtwork(aside)}
        title={live ? aside.station.name : aside.episode.title}
        className="player-cover"
        placeholder={<Icon name={live ? "radio" : "podcasts"} size={26} />}
      />
      <div className="player-track">
        <strong>{live ? aside.station.name : aside.episode.title}</strong>
        {live ? (
          <span className="badge live-badge">{t("player.live")}</span>
        ) : (
          <span>{aside.episode.channel_title}</span>
        )}
        {player.asideError ? (
          <small role="alert">
            {live ? t("player.liveError") : t("podcasts.playbackError")}
          </small>
        ) : null}
      </div>
      <div className="player-controls">
        <button
          type="button"
          className="primary"
          onClick={player.toggleAside}
          aria-label={primaryLabel}
        >
          <Icon
            name={player.asidePlaying ? (live ? "stop" : "pause") : "play"}
            size={24}
          />
        </button>
        <button
          type="button"
          onClick={() => player.stopAside()}
          aria-label={t("player.closeAside")}
        >
          <Icon name="close" />
        </button>
      </div>
      {transport.progress ? <AsideScrubber /> : null}
      <div className="player-aside">
        <LevelControls />
      </div>
    </footer>
  );
}

/** Render the persistent controls for the currently selected track. */
export function PlayerBar() {
  const player = usePlayer();
  const { t } = useI18n();
  if (player.aside) return <AsideBar aside={player.aside} />;
  if (!player.current) return null;
  const cover = (
    <Artwork
      artworkId={player.current.artwork_hash}
      title={player.current.title}
      className="player-cover"
    />
  );
  const repeatLabel =
    player.repeat === "off"
      ? t("player.repeatOff")
      : player.repeat === "all"
        ? t("player.repeatAll")
        : t("player.repeatOne");
  return (
    <footer className="player">
      {/* The cover is the way back to what is playing. Without it the only
          route to the album of the current track is to remember its name and
          search for it. */}
      {player.current.album_id ? (
        <Link
          to="/albums/$albumId"
          params={{ albumId: player.current.album_id }}
          aria-label={`${t("player.openAlbum")}: ${player.current.album ?? player.current.title}`}
        >
          {cover}
        </Link>
      ) : (
        cover
      )}
      <div className="player-track">
        <strong>{player.current.title}</strong>
        <span>{player.current.artist ?? t("common.unknownArtist")}</span>
        {player.error ? <small role="alert">{t("player.error")}</small> : null}
      </div>
      <div className="player-controls">
        <button
          type="button"
          className={player.shuffle ? "mode on" : "mode"}
          onClick={player.toggleShuffle}
          aria-label={t("player.shuffle")}
          aria-pressed={player.shuffle}
        >
          <Icon name="random" size={18} />
        </button>
        <button
          type="button"
          onClick={player.previous}
          aria-label={t("player.previous")}
        >
          <Icon name="previous" />
        </button>
        <button
          type="button"
          className="primary"
          onClick={player.toggle}
          aria-label={player.playing ? t("player.pause") : t("player.play")}
        >
          <Icon name={player.playing ? "pause" : "play"} size={24} />
        </button>
        <button
          type="button"
          onClick={player.next}
          aria-label={t("player.next")}
        >
          <Icon name="next" />
        </button>
        {/* Three states on one button, so its label carries the current one
            rather than the action — a screen reader hearing "repeat" alone
            could not tell which of the three is on. */}
        <button
          type="button"
          className={player.repeat === "off" ? "mode" : "mode on"}
          onClick={player.cycleRepeat}
          aria-label={repeatLabel}
        >
          <Icon
            name={player.repeat === "one" ? "repeatOne" : "repeat"}
            size={18}
          />
        </button>
      </div>
      <QueueScrubber />
      <div className="player-aside">
        <LevelControls />
      </div>
    </footer>
  );
}
