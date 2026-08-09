import { Link, useNavigate } from "@tanstack/react-router";
import { type FormEvent, useEffect, useMemo, useState } from "react";

import {
  type Album,
  type AlbumDetail,
  type Artist,
  type ArtistDetail,
  addLibrary,
  appendToPlaylist,
  authorize,
  bootstrapAdmin,
  createPlaylist,
  createShare,
  createUser,
  currentUser,
  deletePlaylist,
  deleteShare,
  formatDuration,
  getAlbum,
  getArtist,
  getTrack,
  isAllowedRedirect,
  listAlbums,
  listArtists,
  listFavorites,
  listLibraries,
  listPlaylists,
  listShares,
  listUsers,
  login,
  type Playlist,
  type SearchResult,
  type Share,
  type Song,
  safeInternalPath,
  search,
  setFavorite,
  setSubsonicCredential,
  setUserDisabled,
  setupRequired,
  startScan,
  type User,
} from "./api";
import { usePlayer } from "./player";

/** Resolves a promise into render state, with the error surfaced rather than swallowed. */
function useAsync<T>(load: () => Promise<T>, deps: unknown[]) {
  const [value, setValue] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    setValue(null);
    setError(null);
    load().then(
      (result) => !cancelled && setValue(result),
      (cause: unknown) =>
        !cancelled &&
        setError(cause instanceof Error ? cause.message : "error"),
    );
    return () => {
      cancelled = true;
    };
    // Placed here, not above useEffect: the rule fires on the dependency
    // argument, and biome only accepts the suppression on that line.
    // biome-ignore lint/correctness/useExhaustiveDependencies: the caller supplies the dependency list, which is the point of this hook
  }, deps);
  return { value, error };
}

function Loading({ error }: { error: string | null }) {
  return <p className="muted">{error ? `Failed: ${error}` : "Loading…"}</p>;
}

/**
 * Provides administrator setup and user authentication, then redirects to the requested internal path or the home page.
 */
export function LoginPage() {
  const navigate = useNavigate();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [setup, setSetup] = useState(false);

  useEffect(() => {
    void setupRequired().then(setSetup, () => setSetup(false));
  }, []);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      if (setup) {
        try {
          await bootstrapAdmin(username, password);
          setSetup(false);
        } catch {
          setError(
            "Setup failed. Use a valid username and at least 12 password characters.",
          );
          return;
        }
      }
      try {
        await login(username, password);
      } catch {
        setError("Wrong username or password.");
        return;
      }
      const next = safeInternalPath(
        sessionStorage.getItem("waveflow.after-login"),
      );
      sessionStorage.removeItem("waveflow.after-login");
      if (next) window.location.assign(next);
      else await navigate({ to: "/" });
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="centered">
      <form className="card" onSubmit={submit}>
        <h1>{setup ? "Create the administrator" : "WaveFlow"}</h1>
        {setup ? (
          <p className="muted">
            This is a new server. Choose the first administrator account.
          </p>
        ) : null}
        <label>
          Username
          <input
            value={username}
            onChange={(event) => setUsername(event.target.value)}
            autoComplete="username"
            required
          />
        </label>
        <label>
          Password
          <input
            type="password"
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            autoComplete={setup ? "new-password" : "current-password"}
            minLength={setup ? 12 : undefined}
            required
          />
        </label>
        {error && <p className="error">{error}</p>}
        <button type="submit" disabled={busy}>
          {busy ? "Please wait…" : setup ? "Create and sign in" : "Sign in"}
        </button>
      </form>
    </main>
  );
}

function AlbumGrid({ albums }: { albums: Album[] }) {
  if (albums.length === 0) return <p className="muted">No albums yet.</p>;
  return (
    <ul className="grid">
      {albums.map((album) => (
        <li key={album.id}>
          <Link to="/albums/$albumId" params={{ albumId: album.id }}>
            <div className="cover" aria-hidden="true">
              {album.title.slice(0, 1)}
            </div>
            <strong>{album.title}</strong>
            <span className="muted">
              {album.artist ?? "Various artists"}
              {album.year ? ` · ${album.year}` : ""}
            </span>
          </Link>
        </li>
      ))}
    </ul>
  );
}

/**
 * Displays the available albums with loading and error states.
 *
 * @returns The albums page content.
 */
export function AlbumsPage() {
  const { value, error } = useAsync(listAlbums, []);
  if (!value) return <Loading error={error} />;
  return (
    <section>
      <h2>Albums</h2>
      <AlbumGrid albums={value} />
    </section>
  );
}

/**
 * Renders a table of songs with playback controls, metadata, and favourite toggles.
 *
 * @param songs - The songs to display.
 */
export function SongTable({ songs }: { songs: Song[] }) {
  const player = usePlayer();
  const [stars, setStars] = useState<Record<string, boolean>>({});

  async function toggleStar(song: Song) {
    const on = !(stars[song.id] ?? song.starred_at !== null);
    setStars((previous) => ({ ...previous, [song.id]: on }));
    try {
      await setFavorite("track", song.id, on);
    } catch {
      setStars((previous) => ({ ...previous, [song.id]: !on }));
    }
  }

  return (
    <div className="songs-scroll">
      <table className="songs">
        <tbody>
          {songs.map((song, position) => {
            const starred = stars[song.id] ?? song.starred_at !== null;
            const active = player.current?.id === song.id;
            return (
              <tr key={song.id} className={active ? "active" : undefined}>
                <td className="index">{song.track ?? position + 1}</td>
                <td>
                  <button
                    type="button"
                    className="link"
                    onClick={() => player.play(songs, position)}
                  >
                    {song.title}
                  </button>
                </td>
                <td className="muted">{song.artist}</td>
                <td className="muted">{formatDuration(song.duration_ms)}</td>
                <td>
                  <button
                    type="button"
                    className="star"
                    onClick={() => void toggleStar(song)}
                    aria-label={starred ? "Remove favourite" : "Add favourite"}
                    aria-pressed={starred}
                  >
                    {starred ? "★" : "☆"}
                  </button>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

/**
 * Displays the signed-in user's favourite tracks.
 *
 * @returns The favourites page content.
 */
export function FavoritesPage() {
  const { value, error } = useAsync(async () => {
    const favorites = await listFavorites();
    const tracks = favorites.filter((item) => item.entity_type === "track");
    const resolved = await Promise.allSettled(
      tracks.map((item) => getTrack(item.entity_id)),
    );
    return resolved.flatMap((result) =>
      result.status === "fulfilled" ? [result.value] : [],
    );
  }, []);
  if (!value) return <Loading error={error} />;
  return (
    <section>
      <PageHeader title="Favourites" detail={`${value.length} saved tracks`} />
      {value.length ? (
        <SongTable songs={value} />
      ) : (
        <EmptyState message="Star a track from an album or search result and it will appear here." />
      )}
    </section>
  );
}

/**
 * Displays playlists and provides controls to create, play, update, and delete them.
 *
 * @returns The playlists page content.
 */
export function PlaylistsPage() {
  const player = usePlayer();
  const [revision, setRevision] = useState(0);
  const [name, setName] = useState("");
  const [mutationError, setMutationError] = useState<string | null>(null);
  const { value, error } = useAsync(listPlaylists, [revision]);

  async function create(event: FormEvent) {
    event.preventDefault();
    setMutationError(null);
    try {
      await createPlaylist(name);
      setName("");
      setRevision((value) => value + 1);
    } catch {
      setMutationError("The playlist could not be created.");
    }
  }

  async function addQueue(playlist: Playlist) {
    if (!player.queue.length) return;
    setMutationError(null);
    try {
      await appendToPlaylist(
        playlist.id,
        player.queue.map((song) => song.id),
      );
      setRevision((value) => value + 1);
    } catch {
      setMutationError("The queue could not be added to this playlist.");
    }
  }

  async function removePlaylist(id: string) {
    setMutationError(null);
    try {
      await deletePlaylist(id);
      setRevision((value) => value + 1);
    } catch {
      setMutationError("The playlist could not be deleted.");
    }
  }

  if (!value) return <Loading error={error} />;
  return (
    <section>
      <PageHeader title="Playlists" detail={`${value.length} collections`} />
      <form className="inline-form" onSubmit={(event) => void create(event)}>
        <input
          value={name}
          onChange={(event) => setName(event.target.value)}
          placeholder="New playlist name"
          aria-label="New playlist name"
          required
        />
        <button type="submit">Create playlist</button>
      </form>
      {mutationError ? <p className="error">{mutationError}</p> : null}
      {value.length ? (
        <div className="stack">
          {value.map((playlist) => (
            <article className="collection" key={playlist.id}>
              <header className="collection-header">
                <div>
                  <h3>{playlist.name}</h3>
                  <span className="muted">{playlist.songs.length} tracks</span>
                </div>
                <div className="actions">
                  <button
                    type="button"
                    onClick={() => player.play(playlist.songs, 0)}
                    disabled={!playlist.songs.length}
                  >
                    Play
                  </button>
                  <button
                    type="button"
                    onClick={() => void addQueue(playlist)}
                    disabled={!player.queue.length}
                  >
                    Add queue
                  </button>
                  <button
                    type="button"
                    className="danger"
                    onClick={() => void removePlaylist(playlist.id)}
                  >
                    Delete
                  </button>
                </div>
              </header>
              {playlist.songs.length ? (
                <SongTable songs={playlist.songs} />
              ) : (
                <p className="muted">This playlist is empty.</p>
              )}
            </article>
          ))}
        </div>
      ) : (
        <EmptyState message="Create a playlist to keep a set of tracks together." />
      )}
    </section>
  );
}

/**
 * Displays the synchronized playback queue and provides controls to play, clear, or remove tracks.
 */
export function QueuePage() {
  const player = usePlayer();
  const queueKeys = useMemo(() => {
    const occurrences = new Map<string, number>();
    return player.queue.map((song) => {
      const occurrence = occurrences.get(song.id) ?? 0;
      occurrences.set(song.id, occurrence + 1);
      return `${song.id}-${occurrence}`;
    });
  }, [player.queue]);
  return (
    <section>
      <PageHeader
        title="Queue"
        detail={`${player.queue.length} tracks synchronized with your account`}
      />
      {player.queue.length ? (
        <>
          <div className="actions section-actions">
            <button type="button" onClick={() => player.play(player.queue, 0)}>
              Play from start
            </button>
            <button type="button" className="danger" onClick={player.clear}>
              Clear queue
            </button>
          </div>
          <ol className="queue-list">
            {player.queue.map((song, position) => (
              <li
                key={queueKeys[position]}
                className={player.index === position ? "active" : undefined}
              >
                <button
                  type="button"
                  className="link queue-title"
                  onClick={() => player.play(player.queue, position)}
                >
                  {song.title}
                </button>
                <span className="muted">{song.artist ?? "Unknown artist"}</span>
                <span className="muted">
                  {formatDuration(song.duration_ms)}
                </span>
                <button type="button" onClick={() => player.remove(position)}>
                  Remove
                </button>
              </li>
            ))}
          </ol>
        </>
      ) : (
        <EmptyState message="Play an album or playlist to build your synchronized queue." />
      )}
    </section>
  );
}

/**
 * Displays and manages public links for the current music queue.
 */
export function SharesPage() {
  const player = usePlayer();
  const [description, setDescription] = useState("");
  const [createdShare, setCreatedShare] = useState<Share | null>(null);
  const [revision, setRevision] = useState(0);
  const [mutationError, setMutationError] = useState<string | null>(null);
  const { value, error } = useAsync(listShares, [revision]);
  const createdShareNotice = createdShare?.url ? (
    <p>
      This link is shown only once:{" "}
      <a className="resource-url" href={createdShare.url}>
        {createdShare.url}
      </a>
    </p>
  ) : null;
  if (!value) {
    return (
      <>
        {createdShareNotice}
        <Loading error={error} />
      </>
    );
  }

  async function shareQueue(event: FormEvent) {
    event.preventDefault();
    setMutationError(null);
    try {
      const share = await createShare(
        player.queue.map((song) => song.id),
        description,
      );
      setCreatedShare(share);
      setDescription("");
      setRevision((value) => value + 1);
    } catch {
      setMutationError("The share could not be created.");
    }
  }

  async function removeShare(id: string) {
    setMutationError(null);
    try {
      await deleteShare(id);
      setCreatedShare((current) => (current?.id === id ? null : current));
      setRevision((value) => value + 1);
    } catch {
      setMutationError("The share could not be deleted.");
    }
  }

  return (
    <section>
      <PageHeader title="Shares" detail="Public links for selected music" />
      <form
        className="inline-form"
        onSubmit={(event) => void shareQueue(event)}
      >
        <input
          value={description}
          onChange={(event) => setDescription(event.target.value)}
          placeholder="Description"
          aria-label="Share description"
          required
        />
        <button type="submit" disabled={!player.queue.length}>
          Share current queue
        </button>
      </form>
      {!player.queue.length ? (
        <p className="muted">Add tracks to the queue before creating a link.</p>
      ) : null}
      {mutationError ? <p className="error">{mutationError}</p> : null}
      {createdShareNotice}
      {value.length ? (
        <ul className="resource-list">
          {value.map((share) => (
            <li key={share.id}>
              <div>
                <strong>{share.description ?? "Music share"}</strong>
                {share.url ? (
                  <a className="muted resource-url" href={share.url}>
                    {share.url}
                  </a>
                ) : null}
              </div>
              <span className="muted">{share.track_ids.length} tracks</span>
              <button
                type="button"
                className="danger"
                onClick={() => void removeShare(share.id)}
              >
                Delete
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <EmptyState message="No public links have been created." />
      )}
    </section>
  );
}

/**
 * Provides a form for rotating a user's dedicated Subsonic credential.
 *
 * @param user - The user whose credential will be rotated
 */
function CredentialForm({ user }: { user: User }) {
  const [password, setPassword] = useState("");
  const [apiKey, setApiKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  async function submit(event: FormEvent) {
    event.preventDefault();
    setError(null);
    try {
      const result = await setSubsonicCredential(user.username, password);
      setApiKey(result.api_key);
      setPassword("");
    } catch {
      setError("Credential rotation failed.");
    }
  }
  return (
    <form className="credential-form" onSubmit={(event) => void submit(event)}>
      <input
        type="password"
        value={password}
        onChange={(event) => setPassword(event.target.value)}
        placeholder="Dedicated Subsonic password"
        aria-label={`Subsonic password for ${user.username}`}
        minLength={12}
        required
      />
      <button type="submit">Rotate credential</button>
      {apiKey ? (
        <output className="secret-output">
          Copy this API key now: <code>{apiKey}</code>
        </output>
      ) : null}
      {error ? <span className="error">{error}</span> : null}
    </form>
  );
}

/**
 * Provides administrative controls for managing libraries, scans, and user accounts.
 */
export function AdminPage() {
  const signedInUser = currentUser();
  const [revision, setRevision] = useState(0);
  const [notice, setNotice] = useState<string | null>(null);
  const [adminError, setAdminError] = useState<string | null>(null);
  const [libraryName, setLibraryName] = useState("");
  const [libraryPath, setLibraryPath] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [libraryBusy, setLibraryBusy] = useState(false);
  const [userBusy, setUserBusy] = useState(false);
  const { value, error } = useAsync(
    () => Promise.all([listLibraries(), listUsers()]),
    [revision],
  );
  if (!value) return <Loading error={error} />;
  const [libraries, users] = value;

  async function registerLibrary(event: FormEvent) {
    event.preventDefault();
    setLibraryBusy(true);
    setAdminError(null);
    setNotice(null);
    try {
      const result = await addLibrary(libraryName, libraryPath, "private");
      setLibraryName("");
      setLibraryPath("");
      setNotice(`Initial scan ${result.scan_id} queued.`);
      setRevision((value) => value + 1);
    } catch {
      setAdminError("The library path could not be registered.");
    } finally {
      setLibraryBusy(false);
    }
  }

  async function addUser(event: FormEvent) {
    event.preventDefault();
    setUserBusy(true);
    setAdminError(null);
    setNotice(null);
    try {
      await createUser(username, password, "user");
      setUsername("");
      setPassword("");
      setRevision((value) => value + 1);
    } catch {
      setAdminError("The account could not be created.");
    } finally {
      setUserBusy(false);
    }
  }

  async function toggleUser(user: User) {
    setAdminError(null);
    setNotice(null);
    try {
      await setUserDisabled(user.username, !user.disabled);
      setRevision((value) => value + 1);
    } catch {
      setAdminError("The account status could not be changed.");
    }
  }

  async function scanLibrary(libraryId: string) {
    setAdminError(null);
    setNotice(null);
    try {
      const result = await startScan(libraryId);
      setNotice(`Scan ${result.scan_id} queued.`);
    } catch {
      setAdminError("The scan could not be started.");
    }
  }

  return (
    <section>
      <PageHeader
        title="Administration"
        detail="Libraries, scans and accounts"
      />
      {notice ? <p className="notice">{notice}</p> : null}
      {adminError ? <p className="error">{adminError}</p> : null}
      <div className="admin-grid">
        <article className="admin-panel">
          <h3>Libraries</h3>
          <form
            className="stacked-form"
            onSubmit={(event) => void registerLibrary(event)}
          >
            <input
              value={libraryName}
              onChange={(event) => setLibraryName(event.target.value)}
              placeholder="Library name"
              required
            />
            <input
              value={libraryPath}
              onChange={(event) => setLibraryPath(event.target.value)}
              placeholder="Absolute server folder path"
              required
            />
            <button type="submit" disabled={libraryBusy}>
              {libraryBusy ? "Registering…" : "Register and scan"}
            </button>
          </form>
          <ul className="resource-list compact">
            {libraries.map((library) => (
              <li key={library.id}>
                <div>
                  <strong>{library.name}</strong>
                  <span className="muted">{library.visibility}</span>
                </div>
                <button
                  type="button"
                  onClick={() => void scanLibrary(library.id)}
                >
                  Scan now
                </button>
              </li>
            ))}
          </ul>
        </article>
        <article className="admin-panel">
          <h3>Accounts</h3>
          <form
            className="stacked-form"
            onSubmit={(event) => void addUser(event)}
          >
            <input
              value={username}
              onChange={(event) => setUsername(event.target.value)}
              placeholder="Username"
              autoComplete="off"
              required
            />
            <input
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              placeholder="Web password, 12 characters minimum"
              minLength={12}
              autoComplete="new-password"
              required
            />
            <button type="submit" disabled={userBusy}>
              {userBusy ? "Creating…" : "Create account"}
            </button>
          </form>
        </article>
      </div>
      <div className="stack">
        {users.map((user) => (
          <article className="user-row" key={user.id}>
            <header>
              <div>
                <strong>{user.username}</strong>
                <span className="muted">
                  {user.role} · {user.folder_ids.length} libraries
                </span>
              </div>
              <button
                type="button"
                onClick={() => void toggleUser(user)}
                disabled={user.id === signedInUser?.id}
                title={
                  user.id === signedInUser?.id
                    ? "You cannot disable your current account"
                    : undefined
                }
              >
                {user.disabled ? "Enable" : "Disable"}
              </button>
            </header>
            <CredentialForm user={user} />
          </article>
        ))}
      </div>
    </section>
  );
}

/**
 * Displays a page heading with supporting detail text.
 *
 * @param title - The page heading
 * @param detail - The supporting text displayed below the heading
 */
function PageHeader({ title, detail }: { title: string; detail: string }) {
  return (
    <header className="page-header">
      <h2>{title}</h2>
      <p className="muted">{detail}</p>
    </header>
  );
}

/**
 * Displays a message for an empty content state.
 *
 * @param message - The message to display
 */
function EmptyState({ message }: { message: string }) {
  return (
    <div className="empty-state">
      <p>{message}</p>
    </div>
  );
}

export function AlbumPage({ albumId }: { albumId: string }) {
  const { value, error } = useAsync<AlbumDetail>(
    () => getAlbum(albumId),
    [albumId],
  );
  if (!value) return <Loading error={error} />;
  return (
    <section>
      <header className="detail-header">
        <div className="cover large" aria-hidden="true">
          {value.title.slice(0, 1)}
        </div>
        <div>
          <h2>{value.title}</h2>
          <p className="muted">
            {value.artist ?? "Various artists"}
            {value.year ? ` · ${value.year}` : ""} · {value.songs.length} tracks
          </p>
        </div>
      </header>
      <SongTable songs={value.songs} />
    </section>
  );
}

export function ArtistsPage() {
  const { value, error } = useAsync<Artist[]>(listArtists, []);
  if (!value) return <Loading error={error} />;
  if (value.length === 0) return <p className="muted">No artists yet.</p>;
  return (
    <section>
      <h2>Artists</h2>
      <ul className="list">
        {value.map((artist) => (
          <li key={artist.id}>
            <Link to="/artists/$artistId" params={{ artistId: artist.id }}>
              {artist.name}
            </Link>
            <span className="muted">
              {artist.album_count} album{artist.album_count === 1 ? "" : "s"}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}

export function ArtistPage({ artistId }: { artistId: string }) {
  const { value, error } = useAsync<ArtistDetail>(
    () => getArtist(artistId),
    [artistId],
  );
  if (!value) return <Loading error={error} />;
  return (
    <section>
      <h2>{value.name}</h2>
      <AlbumGrid albums={value.albums} />
    </section>
  );
}

export function SearchPage() {
  const [query, setQuery] = useState("");
  const [submitted, setSubmitted] = useState("");
  const { value, error } = useAsync<SearchResult | null>(
    () => (submitted ? search(submitted) : Promise.resolve(null)),
    [submitted],
  );

  return (
    <section>
      <h2>Search</h2>
      <form
        className="search"
        onSubmit={(event) => {
          event.preventDefault();
          setSubmitted(query.trim());
        }}
      >
        <input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Title, album or artist"
          aria-label="Search query"
        />
        <button type="submit">Search</button>
      </form>
      {submitted && !value && <Loading error={error} />}
      {value && (
        <>
          {value.artists.length > 0 && (
            <>
              <h3>Artists</h3>
              <ul className="list">
                {value.artists.map((artist) => (
                  <li key={artist.id}>
                    <Link
                      to="/artists/$artistId"
                      params={{ artistId: artist.id }}
                    >
                      {artist.name}
                    </Link>
                  </li>
                ))}
              </ul>
            </>
          )}
          {value.albums.length > 0 && (
            <>
              <h3>Albums</h3>
              <AlbumGrid albums={value.albums} />
            </>
          )}
          {value.songs.length > 0 && (
            <>
              <h3>Tracks</h3>
              <SongTable songs={value.songs} />
            </>
          )}
          {value.artists.length === 0 &&
            value.albums.length === 0 &&
            value.songs.length === 0 && <p className="muted">Nothing found.</p>}
        </>
      )}
    </section>
  );
}

/**
 * Presents a consent screen for an Authorization Code + PKCE request.
 *
 * Validates the request parameters and trusted redirect, then either approves
 * the request and follows the server-provided redirect or redirects with an
 * `access_denied` error.
 */
export function AuthorizePage() {
  const params = new URLSearchParams(window.location.search);
  const clientId = params.get("client_id") ?? "";
  const redirectUri = params.get("redirect_uri") ?? "";
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // Refuse the whole screen when the target is one we would not navigate to:
  // failing here is clearer than offering a Cancel button that cannot honour
  // its own redirect.
  const missing =
    !clientId ||
    !redirectUri ||
    !params.get("code_challenge") ||
    !isAllowedRedirect(redirectUri);

  async function approve() {
    setBusy(true);
    setError(null);
    try {
      const response = await authorize({
        client_id: clientId,
        redirect_uri: redirectUri,
        code_challenge: params.get("code_challenge") ?? "",
        code_challenge_method: params.get("code_challenge_method") ?? "S256",
        state: params.get("state"),
        device_name: params.get("device_name") ?? clientId,
      });
      window.location.assign(response.redirect_to);
    } catch {
      setError(
        "The application sent an authorisation request we cannot honour.",
      );
      setBusy(false);
    }
  }

  function deny() {
    if (!isAllowedRedirect(redirectUri)) return;
    const url = new URL(redirectUri);
    url.searchParams.set("error", "access_denied");
    const state = params.get("state");
    if (state) url.searchParams.set("state", state);
    window.location.assign(url.toString());
  }

  if (missing) {
    return (
      <section>
        <h2>Authorisation</h2>
        <p className="error">
          This authorisation link is incomplete or asks to send you somewhere we
          do not trust.
        </p>
      </section>
    );
  }

  return (
    <section className="consent">
      <h2>Authorise {clientId}</h2>
      <p className="muted">
        It will be able to browse your libraries, play your music and manage
        your playlists, favourites and ratings, everything this account can do.
      </p>
      <p className="muted">
        Sending you back to <code>{redirectUri}</code>
      </p>
      {error && <p className="error">{error}</p>}
      <div className="consent-actions">
        <button type="button" onClick={() => void approve()} disabled={busy}>
          {busy ? "Authorising…" : "Authorise"}
        </button>
        <button type="button" onClick={deny} disabled={busy}>
          Cancel
        </button>
      </div>
    </section>
  );
}
