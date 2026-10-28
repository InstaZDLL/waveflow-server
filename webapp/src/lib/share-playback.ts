/**
 * What a share visitor's browser is asked to play.
 *
 * The share stream offers two things and nothing else (RFC-020 decision 5):
 * the original file, or the server's fixed MP3. The original is preferred —
 * it costs the owner no encode, and it seeks by byte range from the first
 * second — so the MP3 is asked for only when the browser says it cannot play
 * the original's format, or when the format is one it has no name for.
 *
 * No React here, and the browser's verdict is passed in, so the choice can be
 * tested without one.
 */

/** The media type a browser is asked about, by the file's suffix. */
const MEDIA_TYPES: Record<string, string> = {
  aac: "audio/aac",
  flac: "audio/flac",
  m4a: "audio/mp4",
  mp3: "audio/mpeg",
  mp4: "audio/mp4",
  oga: "audio/ogg",
  ogg: "audio/ogg",
  opus: 'audio/ogg; codecs="opus"',
  wav: "audio/wav",
  webm: "audio/webm",
};

/** `HTMLMediaElement.canPlayType`: `""`, `"maybe"` or `"probably"`. */
export type CanPlayType = (mediaType: string) => string;

export function shareSource(
  streamUrl: string,
  suffix: string,
  canPlayType: CanPlayType,
): string {
  const mediaType = MEDIA_TYPES[suffix.toLowerCase()];
  if (mediaType && canPlayType(mediaType) !== "") return streamUrl;
  const separator = streamUrl.includes("?") ? "&" : "?";
  return `${streamUrl}${separator}format=mp3`;
}

/** What the share page's one `<audio>` element is given. */
export type SharePlayer = {
  /** The row the element holds: the one chosen, or the first. */
  position: number;
  src: string | undefined;
  /** Only once a row was chosen: opening the page plays nothing by itself. */
  autoPlay: boolean;
  /**
   * `none` until something was chosen, so opening the page fetches no audio
   * and, for a format the browser cannot play, starts no encode.
   */
  preload: "none" | "auto";
};

/**
 * The player for the row chosen, or for the first row when none was.
 *
 * **Never without a source while the share holds a track.** The element had
 * none until a row was touched, so its own play button — the one a visitor
 * reaches for first — had nothing to play: Chrome on Android showed 0:00 /
 * 0:00 and "Unable to play media", while touching a row played. Holding the
 * first row makes that button play what touching the first row plays, from
 * the same address.
 */
export function sharePlayer(
  tracks: ReadonlyArray<{ streamUrl: string; suffix: string }>,
  current: number | null,
  canPlayType: CanPlayType,
): SharePlayer {
  const position = current ?? 0;
  const track = tracks[position];
  return {
    position,
    src: track
      ? shareSource(track.streamUrl, track.suffix, canPlayType)
      : undefined,
    autoPlay: current !== null,
    preload: current === null ? "none" : "auto",
  };
}
