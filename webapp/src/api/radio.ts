import { call } from "./client";

/**
 * An internet radio station the account keeps.
 *
 * Not a {@link Song}: no library, no duration, nothing scanned. The server
 * stores the address and hands it over without ever fetching it, so the
 * browser is what connects — which is also why a station that will not play
 * fails in the player and not in a server response.
 */
export type RadioStation = {
  id: string;
  name: string;
  stream_url: string;
  homepage_url: string | null;
  created_at: number;
  updated_at: number;
};

/** What a station is created or replaced with. */
export type RadioStationDraft = {
  name: string;
  stream_url: string;
  homepage_url?: string | null;
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
