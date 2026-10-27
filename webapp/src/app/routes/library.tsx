// The catalogue's routes, the account's lists and listening, the editor.
import { createRoute } from "@tanstack/react-router";
import { AlbumPage } from "../../pages/library/album-page";
import { AlbumsPage } from "../../pages/library/albums-page";
import { ArtistPage } from "../../pages/library/artist-page";
import { ArtistsPage } from "../../pages/library/artists-page";
import { FavoritesPage } from "../../pages/library/favorites-page";
import { FoldersPage } from "../../pages/library/folders-page";
import { GenrePage } from "../../pages/library/genre-page";
import { GenresPage } from "../../pages/library/genres-page";
import { HistoryPage } from "../../pages/library/history-page";
import { MixPage } from "../../pages/library/mix-page";
import { PlaylistsPage } from "../../pages/library/playlists-page";
import { SearchPage } from "../../pages/library/search-page";
import { SongsPage } from "../../pages/library/songs-page";
import { StatsPage } from "../../pages/library/stats-page";
import { TrackEditorPage } from "../../pages/library/track-editor";
import { authedRoute } from "./root";

export const albumsRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/",
  component: AlbumsPage,
});

export const albumRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/albums/$albumId",
  component: function AlbumRoute() {
    const { albumId } = albumRoute.useParams();
    return <AlbumPage albumId={albumId} />;
  },
});

export const songsRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/songs",
  component: SongsPage,
});

export const artistsRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/artists",
  component: ArtistsPage,
});

export const artistRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/artists/$artistId",
  component: function ArtistRoute() {
    const { artistId } = artistRoute.useParams();
    return <ArtistPage artistId={artistId} />;
  },
});

export const genresRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/genres",
  component: GenresPage,
});

export const genreRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/genres/$genre",
  component: function GenreRoute() {
    const { genre } = genreRoute.useParams();
    return <GenrePage genre={genre} />;
  },
});

export const historyRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/history",
  component: HistoryPage,
});

// RFC-024: this account's listening, and the mixes drawn from it.
export const statsRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/stats",
  component: StatsPage,
});

export const mixRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/mixes/$mixId",
  component: function MixRoute() {
    const { mixId } = mixRoute.useParams();
    return <MixPage mixId={mixId} />;
  },
});

export const searchRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/search",
  component: SearchPage,
});

export const favoritesRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/favourites",
  component: FavoritesPage,
});

export const playlistsRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/playlists",
  component: PlaylistsPage,
});

// The folder is a search parameter rather than a path segment: a folder's
// name may hold any character a file system allows, `$` and `?` included,
// and a parameter carries it whole.
export const foldersRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/folders",
  staticData: { layout: "workspace" },
  // The folder: absent or empty for the root, `unfiled` for the tracks filed
  // nowhere, otherwise a folder id. Optional, so the rail links to the root.
  validateSearch: (search: Record<string, unknown>): { folder?: string } => ({
    folder: typeof search.folder === "string" ? search.folder : undefined,
  }),
  component: function FoldersRoute() {
    const { folder } = foldersRoute.useSearch();
    return <FoldersPage folder={folder ?? ""} />;
  },
});

export const trackEditRoute = createRoute({
  getParentRoute: () => authedRoute,
  path: "/tracks/$trackId/edit",
  component: function TrackEditRoute() {
    const { trackId } = trackEditRoute.useParams();
    return <TrackEditorPage trackId={trackId} />;
  },
});
