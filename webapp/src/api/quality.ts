import type { Song } from "./catalog";
import { call, request } from "./client";

/**
 * The real quality of a file, measured from its audio (RFC-016): what the
 * audio says against what the container claims. Worst first, which is the
 * order the server lists and counts them in.
 */
export type QualityVerdict =
  | "lossy_source"
  | "upsampled"
  | "padded"
  | "unreadable"
  | "silent"
  | "pending"
  | "consistent"
  | "lossy"
  | "not_measured";

export const QUALITY_VERDICTS: QualityVerdict[] = [
  "lossy_source",
  "upsampled",
  "padded",
  "unreadable",
  "silent",
  "pending",
  "consistent",
  "lossy",
  "not_measured",
];

export type QualitySummary = {
  /** `false` when the operator turned the analysis off. */
  enabled: boolean;
  /** Available lossless files: the ones the analysis is about. */
  lossless: number;
  /** Of those, how many have a current measurement. */
  measured: number;
  verdicts: Array<{ verdict: QualityVerdict; count: number }>;
};

/** One file's verdict, and the numbers behind it. */
export type TrackQuality = {
  verdict: QualityVerdict;
  /** Where the highs stop, in hertz; `null` with no wall. */
  cutoff_hz: number | null;
  wall_db: number | null;
  nyquist_hz: number | null;
  /** How many bits the samples use; `null` for floating point. */
  effective_bits: number | null;
  analyzed_at: number | null;
};

export type QualityTrack = Song & { quality: TrackQuality };

/** One track's quality, with the averaged spectrum it was judged on. */
export type TrackQualityDetail = TrackQuality & {
  /** 256 bands from 0 Hz to Nyquist, in dB under the loudest one. */
  spectrum: number[] | null;
};

/** How many files one page of the list carries. */
export const QUALITY_TRACKS_PAGE = 200;

export const getQualitySummary = (libraryId: string) =>
  call<QualitySummary>(`/api/v2/libraries/${libraryId}/quality`);

export const listQualityTracks = (
  libraryId: string,
  filter: { verdict?: QualityVerdict; codec?: string },
  offset: number,
  limit = QUALITY_TRACKS_PAGE,
  signal?: AbortSignal,
) => {
  const query = new URLSearchParams({
    offset: String(offset),
    limit: String(limit),
  });
  if (filter.verdict) query.set("verdict", filter.verdict);
  if (filter.codec) query.set("codec", filter.codec);
  return call<QualityTrack[]>(
    `/api/v2/libraries/${libraryId}/quality/tracks?${query}`,
    { signal },
  );
};

/**
 * Drops the library's measurements and queues its files again. Owners and
 * managers only; anyone else is answered the `404` of a foreign library.
 */
export const requestQualityAnalysis = (libraryId: string) =>
  call<void>(`/api/v2/libraries/${libraryId}/quality/analysis`, {
    method: "POST",
  });

export const getTrackQuality = (trackId: string) =>
  call<TrackQualityDetail>(`/api/v2/tracks/${trackId}/quality`);

/**
 * The track's spectrogram, drawn on demand, as an object URL for `<img>`.
 *
 * Fetched with the bearer token like artwork, since `<img src>` cannot send
 * one. The caller revokes the URL when it is done with it. A `429` means a
 * picture is already being drawn for this account, and surfaces as the
 * `ApiError` it is.
 */
export async function spectrogramUrl(trackId: string): Promise<string> {
  const response = await request(`/api/v2/tracks/${trackId}/spectrogram`);
  return URL.createObjectURL(await response.blob());
}
