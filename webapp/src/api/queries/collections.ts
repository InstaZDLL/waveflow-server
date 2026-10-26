// What an account keeps: playlists, shares, bookmarks, radio, podcasts.

import { queryOptions } from "@tanstack/react-query";
import {
  getPodcastChannel,
  getPodcastOverview,
  getSmartPlaylist,
  listBookmarks,
  listRadioStations,
  listShares,
  listSmartPlaylists,
  listVisiblePlaylists,
} from "../index";

export const playlistsQuery = () =>
  queryOptions({ queryKey: ["playlists"], queryFn: listVisiblePlaylists });

export const sharesQuery = () =>
  queryOptions({ queryKey: ["shares"], queryFn: listShares });

export const bookmarksQuery = () =>
  queryOptions({ queryKey: ["bookmarks"], queryFn: listBookmarks });

export const radioStationsQuery = () =>
  queryOptions({ queryKey: ["radio-stations"], queryFn: listRadioStations });

/**
 * Smart playlists (RFC-025), counted but not loaded: a listing carries what
 * each one answers now, and the tracks are asked for when one is opened.
 */
export const smartPlaylistsQuery = () =>
  queryOptions({ queryKey: ["smart-playlists"], queryFn: listSmartPlaylists });

/** One smart playlist, evaluated by the server at the moment it is asked. */
export const smartPlaylistQuery = (id: string) =>
  queryOptions({
    queryKey: ["smart-playlists", id],
    queryFn: () => getSmartPlaylist(id),
  });

/** The podcasts page: the switch, this account's role, the channels. */
export const podcastsQuery = () =>
  queryOptions({ queryKey: ["podcasts"], queryFn: getPodcastOverview });

/** One channel and its episodes, with this account's position in each. */
export const podcastChannelQuery = (id: string) =>
  queryOptions({
    queryKey: ["podcasts", id],
    queryFn: () => getPodcastChannel(id),
  });
