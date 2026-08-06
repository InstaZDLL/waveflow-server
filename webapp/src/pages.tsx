import { Link, useNavigate } from "@tanstack/react-router";
import { useEffect, useState, type FormEvent } from "react";

import {
  authorize,
  isAllowedRedirect,
  safeInternalPath,
  formatDuration,
  getAlbum,
  getArtist,
  listAlbums,
  listArtists,
  login,
  search,
  setFavorite,
  type Album,
  type AlbumDetail,
  type Artist,
  type ArtistDetail,
  type SearchResult,
  type Song,
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
        !cancelled && setError(cause instanceof Error ? cause.message : "error"),
    );
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  return { value, error };
}

function Loading({ error }: { error: string | null }) {
  return <p className="muted">{error ? `Failed: ${error}` : "Loading…"}</p>;
}

export function LoginPage() {
  const navigate = useNavigate();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await login(username, password);
      const next = safeInternalPath(
        sessionStorage.getItem("waveflow.after-login"),
      );
      sessionStorage.removeItem("waveflow.after-login");
      if (next) window.location.assign(next);
      else await navigate({ to: "/" });
    } catch {
      setError("Wrong username or password.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="centered">
      <form className="card" onSubmit={submit}>
        <h1>WaveFlow</h1>
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
            autoComplete="current-password"
            required
          />
        </label>
        {error && <p className="error">{error}</p>}
        <button type="submit" disabled={busy}>
          {busy ? "Signing in…" : "Sign in"}
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

function SongTable({ songs }: { songs: Song[] }) {
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
 * Consent screen for the native Authorization Code + PKCE flow.
 *
 * The desktop application opens this URL in the system browser with its PKCE
 * parameters; approving posts them back with the browser session attached and
 * follows the redirect the server computes.
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
      setError("The application sent an authorisation request we cannot honour.");
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
        your playlists, favourites and ratings — everything this account can do.
      </p>
      <p className="muted">
        Sending you back to <code>{redirectUri}</code>
      </p>
      {error && <p className="error">{error}</p>}
      <div className="consent-actions">
        <button onClick={() => void approve()} disabled={busy}>
          {busy ? "Authorising…" : "Authorise"}
        </button>
        <button onClick={deny} disabled={busy}>
          Cancel
        </button>
      </div>
    </section>
  );
}
