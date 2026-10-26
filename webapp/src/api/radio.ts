import { call } from "./client";

/**
 * An internet radio station the account keeps.
 *
 * Not a {@link Song}: no library, no duration, nothing scanned. The server
 * stores the address and hands it over without ever fetching it, so the
 * browser is what connects — which is also why a station that will not play
 * fails in the player and not in a server response.
 *
 * Its image is the exception: the server fetches it once, when it is set, into
 * the artwork store, and hands out `artwork_hash`. The browser never loads the
 * image from the station's host (RFC-019's `img-src`), so `image_url` is only
 * what the form shows back.
 */
export type RadioStation = {
  id: string;
  name: string;
  stream_url: string;
  homepage_url: string | null;
  /** The image address the account gave, kept even when it could not be fetched. */
  image_url: string | null;
  /** The fetched image; `null` when there is none, or its fetch failed. */
  artwork_hash: string | null;
  created_at: number;
  updated_at: number;
};

/** What a station is created or replaced with. */
export type RadioStationDraft = {
  name: string;
  stream_url: string;
  homepage_url?: string | null;
  /**
   * Absent, the stored image stays; `null`, it is cleared. The one field a
   * replacement does not clear by omission, so a client that never heard of
   * it — a Subsonic one above all — cannot take a station's image away.
   */
  image_url?: string | null;
};

export const listRadioStations = () =>
  call<RadioStation[]>("/api/v2/radio-stations");

export const createRadioStation = (draft: RadioStationDraft) =>
  call<RadioStation>("/api/v2/radio-stations", {
    method: "POST",
    body: JSON.stringify(draft),
  });

/**
 * A replacement and not a patch: the body carries every field, so leaving the
 * homepage out clears it. The route is a `PUT` because the Subsonic
 * `updateInternetRadioStation` it shares a service with behaves that way, and
 * two doors onto one list must not disagree about what an absent field means.
 */
export const replaceRadioStation = (id: string, draft: RadioStationDraft) =>
  call<RadioStation>(`/api/v2/radio-stations/${id}`, {
    method: "PUT",
    body: JSON.stringify(draft),
  });

export const deleteRadioStation = (id: string) =>
  call<void>(`/api/v2/radio-stations/${id}`, { method: "DELETE" });
