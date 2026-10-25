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
