import { Link, useNavigate } from "@tanstack/react-router";
import {
  type FormEvent,
  type ReactNode,
  useEffect,
  useMemo,
  useState,
} from "react";

import {
  type Album,
  type AlbumDetail,
  type AlbumSort,
  type Artist,
  type ArtistDetail,
  addLibrary,
  appendToPlaylist,
  authorize,
  type Bookmark,
  bootstrapAdmin,
  createPlaylist,
  createShare,
  createUser,
  currentUser,
  deleteBookmark,
  deletePlaylist,
  deleteShare,
  formatDuration,
  type Genre,
  getAlbum,
  getArtist,
  getLyrics,
  getTrack,
  isAllowedRedirect,
  type LyricsLine,
  type LyricsList,
  listAlbums,
  listArtists,
  listBookmarks,
  listFavorites,
  listGenreSongs,
  listGenres,
  listHistory,
  listLibraries,
  listPlaylists,
  listRandomSongs,
  listShares,
  listUsers,
  login,
  type Playlist,
  type SearchResult,
  type Share,
  type Song,
  safeInternalPath,
  search,
  setBookmark,
  setFavorite,
  setRating,
  setSubsonicCredential,
  setUserDisabled,
  setupRequired,
  startScan,
  type User,
} from "./api";
import { Artwork } from "./artwork";
import { type TranslationKey, useI18n } from "./i18n";
import { Icon } from "./icons";
import { usePlayer, usePlayerProgress } from "./player";

const SKELETON_KEYS = [
  "one",
  "two",
  "three",
  "four",
  "five",
  "six",
  "seven",
  "eight",
];

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
  const { t } = useI18n();
  if (error) {
    return (
      <div className="error-state" role="alert">
        <strong>{t("common.loadError")}</strong>
        <p>{error}</p>
      </div>
    );
  }
  return (
    <div
      className="skeleton-grid"
      role="status"
      aria-label={t("common.loading")}
      aria-busy="true"
    >
      {SKELETON_KEYS.map((key) => (
        <div className="skeleton-card" key={key}>
          <span />
          <i />
          <i />
        </div>
      ))}
    </div>
  );
}

export function LoginPage() {
  const navigate = useNavigate();
  const { t } = useI18n();
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
          setError(t("login.setupError"));
          return;
        }
      }
      try {
        await login(username, password);
      } catch {
        setError(t("login.error"));
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
      <div className="login-shell">
        <section className="login-story" aria-hidden="true">
          <span className="eyebrow">{t("login.heroEyebrow")}</span>
          <h1>{t("login.heroTitle")}</h1>
          <div className="signal" />
          <p>{t("login.heroDetail")}</p>
        </section>
        <form className="card login-card" onSubmit={submit}>
          <span className="eyebrow">{t("login.server")}</span>
          <h1>{setup ? t("login.createAdmin") : t("login.welcome")}</h1>
          {setup ? <p className="muted">{t("login.setupDetail")}</p> : null}
          <label>
            {t("login.username")}
            <input
              value={username}
              onChange={(event) => setUsername(event.target.value)}
              autoComplete="username"
              required
            />
          </label>
          <label>
            {t("login.password")}
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
            {busy
              ? t("login.wait")
              : setup
                ? t("login.create")
                : t("login.signIn")}
          </button>
        </form>
      </div>
    </main>
  );
}

function AlbumGrid({ albums }: { albums: Album[] }) {
  const player = usePlayer();
  const { t } = useI18n();
  // The grid holds album summaries, not their tracks, so both actions have to
  // fetch the sleeve first. A failed fetch leaves the queue alone rather than
  // half-filling it.
  //
  // A set and not one id: two albums can be in flight at once, and a single
  // slot let whichever answered first clear the other one's guard. The second
  // album's buttons came back to life while its own request was still out, so
  // a second press on "add to queue" queued the album twice.
  const [pending, setPending] = useState<ReadonlySet<string>>(new Set());

  async function withSongs(album: Album, use: (songs: Song[]) => void) {
    setPending((current) => new Set(current).add(album.id));
    try {
      const detail = await getAlbum(album.id);
      if (detail.songs.length) use(detail.songs);
    } catch {
      // Nothing to report here: the album stays reachable through its link.
    } finally {
      setPending((current) => {
        const next = new Set(current);
        next.delete(album.id);
        return next;
      });
    }
  }

  if (albums.length === 0)
    return <p className="muted">{t("common.noAlbums")}</p>;
  return (
    <ul className="grid">
      {albums.map((album) => (
        <li key={album.id}>
          <Link to="/albums/$albumId" params={{ albumId: album.id }}>
            <Artwork artworkId={album.artwork_hash} title={album.title} />
            <strong>{album.title}</strong>
            <span className="muted">
              {album.artist ?? t("common.variousArtists")}
              {album.year ? ` · ${album.year}` : ""}
            </span>
          </Link>
          <div className="card-actions">
            <button
              type="button"
              className="card-action"
              disabled={pending.has(album.id)}
              aria-label={`${t("card.play")}: ${album.title}`}
              onClick={() =>
                void withSongs(album, (songs) => player.play(songs, 0))
              }
            >
              <Icon name="play" size={16} />
            </button>
            <button
              type="button"
              className="card-action"
              disabled={pending.has(album.id)}
              aria-label={`${t("card.queue")}: ${album.title}`}
              onClick={() => void withSongs(album, player.enqueue)}
            >
              <Icon name="queue" size={16} />
            </button>
          </div>
        </li>
      ))}
    </ul>
  );
}

/**
 * The orders offered on the albums page, in menu order. Four of them narrow the
 * catalogue as well as reordering it — the server answers `frequent`, `recent`
 * and `starred` only for albums that have a play count, a last play or a star,
 * and `byYear` only for those carrying a year — which is why the count in the
 * header is read off the response.
 */
const ALBUM_SORTS: Array<{ value: AlbumSort; labelKey: TranslationKey }> = [
  { value: "alphabeticalByName", labelKey: "browse.sortAlphabeticalByName" },
  {
    value: "alphabeticalByArtist",
    labelKey: "browse.sortAlphabeticalByArtist",
  },
  { value: "newest", labelKey: "browse.sortNewest" },
  { value: "recent", labelKey: "browse.sortRecent" },
  { value: "frequent", labelKey: "browse.sortFrequent" },
  { value: "starred", labelKey: "browse.sortStarred" },
  { value: "byYear", labelKey: "browse.sortByYear" },
];

export function AlbumsPage() {
  const { t } = useI18n();
  const [sort, setSort] = useState<AlbumSort>("alphabeticalByName");
  const [filter, setFilter] = useState("");
  const { value, error } = useAsync(() => listAlbums(sort), [sort]);
  const needle = normalizeFilter(filter);
  const shown = useMemo(
    () =>
      !value || !needle
        ? (value ?? [])
        : value.filter(
            (album) =>
              matches(album.title, needle) || matches(album.artist, needle),
          ),
    [value, needle],
  );

  const controls = (
    <>
      <label className="control">
        <span>{t("browse.sort")}</span>
        <select
          value={sort}
          aria-label={t("browse.sort")}
          onChange={(event) => setSort(event.target.value as AlbumSort)}
        >
          {ALBUM_SORTS.map((option) => (
            <option key={option.value} value={option.value}>
              {t(option.labelKey)}
            </option>
          ))}
        </select>
      </label>
      <FilterField
        label={t("browse.filterAlbums")}
        value={filter}
        onChange={setFilter}
      />
    </>
  );

  if (!value) {
    return (
      <section>
        {/* No detail line: the count it would carry is exactly what is still
            missing, and `Loading` already says whether this is a wait or a
            failure. */}
        <PageHeader title={t("nav.albums")}>{controls}</PageHeader>
        <Loading error={error} />
      </section>
    );
  }
  return (
    <section>
      <PageHeader
        title={t("nav.albums")}
        detail={
          needle
            ? t("browse.matches", { count: shown.length, total: value.length })
            : t("albums.detail", { count: value.length })
        }
      >
        {controls}
      </PageHeader>
      {needle && shown.length === 0 ? (
        <EmptyState message={t("browse.noMatch")} />
      ) : (
        <AlbumGrid albums={shown} />
      )}
    </section>
  );
}

const RATING_STARS = [1, 2, 3, 4, 5];

/**
 * Five stars, where clicking the star already set clears the rating — the
 * server spells that `rating: 0`, and without it a rating could be changed but
 * never taken back.
 *
 * The radio group is the accessible shape for "exactly one of five", and it
 * gives arrow-key selection for free. Each group needs a `name` unique to the
 * row, or every rating on a page would belong to one group.
 */
function StarRating({
  label,
  value,
  name,
  onRate,
}: {
  label: string;
  value: number;
  name: string;
  onRate: (rating: number) => void;
}) {
  const { t } = useI18n();
  return (
    <fieldset className="rating" aria-label={label}>
      {RATING_STARS.map((star) => (
        <label key={star} className={star <= value ? "on" : undefined}>
          <input
            type="radio"
            name={name}
            checked={star === value}
            aria-label={t("rating.stars", { count: star })}
            onChange={() => onRate(star)}
            onClick={() => star === value && onRate(0)}
          />
          <span aria-hidden="true">★</span>
        </label>
      ))}
    </fieldset>
  );
}

export function SongTable({ songs }: { songs: Song[] }) {
  const player = usePlayer();
  const { t } = useI18n();
  const [stars, setStars] = useState<Record<string, boolean>>({});
  const [ratings, setRatings] = useState<Record<string, number>>({});

  async function toggleStar(song: Song) {
    const on = !(stars[song.id] ?? song.starred_at !== null);
    setStars((previous) => ({ ...previous, [song.id]: on }));
    try {
      await setFavorite("track", song.id, on);
    } catch {
      setStars((previous) => ({ ...previous, [song.id]: !on }));
    }
  }

  async function rate(song: Song, rating: number) {
    const previous = ratings[song.id] ?? song.user_rating ?? 0;
    setRatings((current) => ({ ...current, [song.id]: rating }));
    try {
      await setRating("track", song.id, rating);
    } catch {
      setRatings((current) => ({ ...current, [song.id]: previous }));
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
                    className="link song-title"
                    onClick={() => player.play(songs, position)}
                  >
                    <Artwork
                      artworkId={song.artwork_hash}
                      title={song.title}
                      className="song-cover"
                    />
                    <span>
                      {song.title}
                      <small>{song.album}</small>
                    </span>
                  </button>
                </td>
                <td className="muted">{song.artist}</td>
                <td className="muted">{formatDuration(song.duration_ms)}</td>
                <td className="song-marks">
                  <div>
                    <StarRating
                      label={`${t("rating.label")}: ${song.title}`}
                      name={`rating-${song.id}`}
                      value={ratings[song.id] ?? song.user_rating ?? 0}
                      onRate={(rating) => void rate(song, rating)}
                    />
                    <button
                      type="button"
                      className="star"
                      onClick={() => void toggleStar(song)}
                      aria-label={`${
                        starred ? t("favourites.remove") : t("favourites.add")
                      }: ${song.title}`}
                      aria-pressed={starred}
                    >
                      <Icon name="heart" size={16} filled={starred} />
                    </button>
                  </div>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

export function FavoritesPage() {
  const { t } = useI18n();
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
      <PageHeader
        title={t("nav.favourites")}
        detail={t("favourites.detail", { count: value.length })}
      />
      {value.length ? (
        <SongTable songs={value} />
      ) : (
        <EmptyState message={t("favourites.empty")} />
      )}
    </section>
  );
}

export function PlaylistsPage() {
  const player = usePlayer();
  const { t } = useI18n();
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
      setMutationError(t("playlists.createError"));
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
      setMutationError(t("playlists.queueError"));
    }
  }

  async function removePlaylist(id: string) {
    setMutationError(null);
    try {
      await deletePlaylist(id);
      setRevision((value) => value + 1);
    } catch {
      setMutationError(t("playlists.deleteError"));
    }
  }

  if (!value) return <Loading error={error} />;
  return (
    <section>
      <PageHeader
        title={t("nav.playlists")}
        detail={t("playlists.detail", { count: value.length })}
      />
      <form className="inline-form" onSubmit={(event) => void create(event)}>
        <input
          value={name}
          onChange={(event) => setName(event.target.value)}
          placeholder={t("playlists.newName")}
          aria-label={t("playlists.newName")}
          required
        />
        <button type="submit">{t("playlists.create")}</button>
      </form>
      {mutationError ? <p className="error">{mutationError}</p> : null}
      {value.length ? (
        <div className="stack">
          {value.map((playlist) => (
            <article className="collection" key={playlist.id}>
              <header className="collection-header">
                <div>
                  <h3>{playlist.name}</h3>
                  <span className="muted">
                    {t("common.tracks", { count: playlist.songs.length })}
                  </span>
                </div>
                <div className="actions">
                  <button
                    type="button"
                    onClick={() => player.play(playlist.songs, 0)}
                    disabled={!playlist.songs.length}
                  >
                    {t("playlists.play")}
                  </button>
                  <button
                    type="button"
                    onClick={() => void addQueue(playlist)}
                    disabled={!player.queue.length}
                  >
                    {t("playlists.addQueue")}
                  </button>
                  <button
                    type="button"
                    className="danger"
                    onClick={() => void removePlaylist(playlist.id)}
                  >
                    {t("common.delete")}
                  </button>
                </div>
              </header>
              {playlist.songs.length ? (
                <SongTable songs={playlist.songs} />
              ) : (
                <p className="muted">{t("playlists.emptyOne")}</p>
              )}
            </article>
          ))}
        </div>
      ) : (
        <EmptyState message={t("playlists.empty")} />
      )}
    </section>
  );
}

export function QueuePage() {
  const player = usePlayer();
  const { t } = useI18n();
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
        title={t("nav.queue")}
        detail={t("queue.detail", { count: player.queue.length })}
      />
      {player.queue.length ? (
        <>
          <div className="actions section-actions">
            <button type="button" onClick={() => player.play(player.queue, 0)}>
              {t("queue.playStart")}
            </button>
            <button type="button" className="danger" onClick={player.clear}>
              {t("queue.clear")}
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
                <span className="muted">
                  {song.artist ?? t("common.unknownArtist")}
                </span>
                <span className="muted">
                  {formatDuration(song.duration_ms)}
                </span>
                <button
                  type="button"
                  onClick={() => player.remove(position)}
                  aria-label={`${t("queue.remove")}: ${song.title}`}
                >
                  {t("queue.remove")}
                </button>
              </li>
            ))}
          </ol>
        </>
      ) : (
        <EmptyState message={t("queue.empty")} />
      )}
    </section>
  );
}

export function SharesPage() {
  const player = usePlayer();
  const { t } = useI18n();
  const [description, setDescription] = useState("");
  const [createdShare, setCreatedShare] = useState<Share | null>(null);
  const [revision, setRevision] = useState(0);
  const [mutationError, setMutationError] = useState<string | null>(null);
  const { value, error } = useAsync(listShares, [revision]);
  const createdShareNotice = createdShare?.url ? (
    <p>
      {t("shares.once")}{" "}
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
      setMutationError(t("shares.createError"));
    }
  }

  async function removeShare(id: string) {
    setMutationError(null);
    try {
      await deleteShare(id);
      setCreatedShare((current) => (current?.id === id ? null : current));
      setRevision((value) => value + 1);
    } catch {
      setMutationError(t("shares.deleteError"));
    }
  }

  return (
    <section>
      <PageHeader title={t("nav.shares")} detail={t("shares.detail")} />
      <form
        className="inline-form"
        onSubmit={(event) => void shareQueue(event)}
      >
        <input
          value={description}
          onChange={(event) => setDescription(event.target.value)}
          placeholder={t("shares.description")}
          aria-label={t("shares.description")}
          required
        />
        <button type="submit" disabled={!player.queue.length}>
          {t("shares.create")}
        </button>
      </form>
      {!player.queue.length ? (
        <p className="muted">{t("shares.needQueue")}</p>
      ) : null}
      {mutationError ? <p className="error">{mutationError}</p> : null}
      {createdShareNotice}
      {value.length ? (
        <ul className="resource-list">
          {value.map((share) => (
            <li key={share.id}>
              <div>
                <strong>{share.description ?? t("shares.defaultName")}</strong>
                {share.url ? (
                  <a className="muted resource-url" href={share.url}>
                    {share.url}
                  </a>
                ) : null}
              </div>
              <span className="muted">
                {t("common.tracks", { count: share.track_ids.length })}
              </span>
              <button
                type="button"
                className="danger"
                onClick={() => void removeShare(share.id)}
              >
                {t("common.delete")}
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <EmptyState message={t("shares.empty")} />
      )}
    </section>
  );
}

function CredentialForm({ user }: { user: User }) {
  const { t } = useI18n();
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
      setError(t("credential.error"));
    }
  }
  return (
    <form className="credential-form" onSubmit={(event) => void submit(event)}>
      <input
        type="password"
        value={password}
        onChange={(event) => setPassword(event.target.value)}
        placeholder={t("credential.password")}
        aria-label={`${t("credential.password")} — ${user.username}`}
        minLength={12}
        required
      />
      <button type="submit">{t("credential.rotate")}</button>
      {apiKey ? (
        <output className="secret-output">
          {t("credential.copy")} <code>{apiKey}</code>
        </output>
      ) : null}
      {error ? <span className="error">{error}</span> : null}
    </form>
  );
}

/** Plays a set of songs, or adds them to the queue, from a page header. */
function PlaySetActions({ songs }: { songs: Song[] }) {
  const player = usePlayer();
  const { t } = useI18n();
  return (
    <div className="actions">
      <button
        type="button"
        onClick={() => player.play(songs, 0)}
        disabled={!songs.length}
      >
        {t("playlists.play")}
      </button>
      <button
        type="button"
        onClick={() => player.enqueue(songs)}
        disabled={!songs.length}
      >
        {t("playlists.addQueue")}
      </button>
    </div>
  );
}

export function GenresPage() {
  const { t } = useI18n();
  const [filter, setFilter] = useState("");
  const { value, error } = useAsync<Genre[]>(listGenres, []);
  const needle = normalizeFilter(filter);
  const shown = useMemo(
    () =>
      !value || !needle
        ? (value ?? [])
        : value.filter((genre) => matches(genre.name, needle)),
    [value, needle],
  );
  if (!value) return <Loading error={error} />;
  return (
    <section>
      <PageHeader
        title={t("nav.genres")}
        detail={
          needle
            ? t("browse.matches", { count: shown.length, total: value.length })
            : t("genres.detail", { count: value.length })
        }
      >
        <FilterField
          label={t("browse.filterGenres")}
          value={filter}
          onChange={setFilter}
        />
      </PageHeader>
      {value.length === 0 ? (
        <EmptyState message={t("genres.empty")} />
      ) : shown.length === 0 ? (
        <EmptyState message={t("browse.noMatch")} />
      ) : (
        <ul className="list genre-list">
          {shown.map((genre) => (
            <li key={genre.name}>
              <Link to="/genres/$genre" params={{ genre: genre.name }}>
                <strong>{genre.name}</strong>
              </Link>
              <span className="muted">
                {t("common.tracks", { count: genre.song_count })} ·{" "}
                {t("common.albums", { count: genre.album_count })}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

export function GenrePage({ genre }: { genre: string }) {
  const player = usePlayer();
  const { t } = useI18n();
  const [drawing, setDrawing] = useState(false);
  const { value, error } = useAsync<Song[]>(
    () => listGenreSongs(genre),
    [genre],
  );

  /**
   * A shuffle of this genre, drawn by the server. It is a separate route from
   * the listing below rather than a client-side shuffle of it, because
   * `/songs/random` samples the whole genre and this page holds only what it
   * has paged in.
   */
  async function drawRandom() {
    setDrawing(true);
    try {
      const songs = await listRandomSongs(100, genre);
      if (songs.length) player.play(songs, 0);
    } catch {
      // The listing below is still playable; nothing to recover from.
    } finally {
      setDrawing(false);
    }
  }

  if (!value) return <Loading error={error} />;
  return (
    <section>
      <PageHeader
        title={genre}
        detail={t("common.tracks", { count: value.length })}
      >
        <div className="actions">
          <button
            type="button"
            onClick={() => player.play(value, 0)}
            disabled={!value.length}
          >
            {t("playlists.play")}
          </button>
          <button
            type="button"
            onClick={() => void drawRandom()}
            disabled={drawing || !value.length}
          >
            {t("random.fromGenre")}
          </button>
        </div>
      </PageHeader>
      {value.length ? (
        <SongTable songs={value} />
      ) : (
        <EmptyState message={t("genres.emptyOne")} />
      )}
    </section>
  );
}

export function HistoryPage() {
  const { t } = useI18n();
  const { value, error } = useAsync(async () => {
    const plays = await listHistory(200);
    // The route answers plays, not songs, and the same track can appear many
    // times. Keeping the first sighting of each id gives "recently played" in
    // the order it was last played, and asks for each track once.
    const seen = new Set<string>();
    const ordered = plays.filter((play) => {
      if (seen.has(play.track_id)) return false;
      seen.add(play.track_id);
      return true;
    });
    const resolved = await Promise.allSettled(
      ordered.map((play) => getTrack(play.track_id)),
    );
    return resolved.flatMap((result) =>
      result.status === "fulfilled" ? [result.value] : [],
    );
  }, []);
  if (!value) return <Loading error={error} />;
  return (
    <section>
      <PageHeader
        title={t("nav.history")}
        detail={t("history.detail", { count: value.length })}
      >
        {value.length ? <PlaySetActions songs={value} /> : null}
      </PageHeader>
      {value.length ? (
        <SongTable songs={value} />
      ) : (
        <EmptyState message={t("history.empty")} />
      )}
    </section>
  );
}

export function RandomPage() {
  const { t } = useI18n();
  const [draw, setDraw] = useState(0);
  const { value, error } = useAsync<Song[]>(() => listRandomSongs(100), [draw]);
  if (!value) return <Loading error={error} />;
  return (
    <section>
      <PageHeader
        title={t("nav.random")}
        detail={t("random.detail", { count: value.length })}
      >
        <div className="actions">
          <PlaySetActions songs={value} />
          <button type="button" onClick={() => setDraw((n) => n + 1)}>
            {t("random.again")}
          </button>
        </div>
      </PageHeader>
      {value.length ? (
        <SongTable songs={value} />
      ) : (
        <EmptyState message={t("random.empty")} />
      )}
    </section>
  );
}

/**
 * The line a synced lyric sheet is on at `seconds`. Lines carry their own start
 * in milliseconds and arrive in order, so this is the last one already begun;
 * `-1` before the first. An unsynced sheet has no starts and never highlights.
 */
export function currentLyricLine(lines: LyricsLine[], seconds: number): number {
  let current = -1;
  for (const [index, line] of lines.entries()) {
    if (line.start === undefined || line.start > seconds * 1000) break;
    current = index;
  }
  return current;
}

function Lyrics({ trackId }: { trackId: string }) {
  const { t } = useI18n();
  const progress = usePlayerProgress();
  const { value, error } = useAsync<LyricsList>(
    () => getLyrics(trackId),
    [trackId],
  );
  const sheet = value?.structured_lyrics[0];
  const active = sheet?.synced
    ? currentLyricLine(sheet.line, progress.position)
    : -1;
  // A refrain repeats word for word and an unsynced sheet has no start to key
  // on, so the line's text alone is not unique. Numbering the repetitions is
  // what the queue does with the same problem.
  const keys = useMemo(() => {
    const seen = new Map<string, number>();
    return (sheet?.line ?? []).map((line) => {
      const occurrence = seen.get(line.value) ?? 0;
      seen.set(line.value, occurrence + 1);
      return `${line.value}-${occurrence}`;
    });
  }, [sheet]);

  if (error) return <p className="muted">{t("lyrics.none")}</p>;
  if (!value) return <p className="muted">{t("common.loading")}</p>;
  if (!sheet || sheet.line.length === 0)
    return <p className="muted">{t("lyrics.none")}</p>;
  return (
    <div className="lyrics">
      <h3>
        {sheet.displayTitle}
        {sheet.displayArtist ? <small>{sheet.displayArtist}</small> : null}
      </h3>
      <ol aria-label={t("lyrics.label")}>
        {sheet.line.map((line, index) => (
          <li
            key={keys[index]}
            className={index === active ? "on" : undefined}
            aria-current={index === active ? "true" : undefined}
          >
            {line.value || " "}
          </li>
        ))}
      </ol>
    </div>
  );
}

export function PlayingPage() {
  const player = usePlayer();
  const progress = usePlayerProgress();
  const { t } = useI18n();
  const [revision, setRevision] = useState(0);
  const [busy, setBusy] = useState(false);
  const { value: bookmarks } = useAsync<Bookmark[]>(listBookmarks, [revision]);
  const current = player.current;

  /**
   * One bookmark per track, replaced rather than added — so this button reads
   * "save" whether or not the track already has one, and saving again simply
   * moves it to where the head is now.
   */
  async function saveHere(song: Song) {
    setBusy(true);
    try {
      await setBookmark(song.id, progress.position * 1000);
      setRevision((n) => n + 1);
    } catch {
      // Nothing is lost: the head has not moved and the old bookmark stands.
    } finally {
      setBusy(false);
    }
  }

  async function forget(trackId: string) {
    try {
      await deleteBookmark(trackId);
      setRevision((n) => n + 1);
    } catch {
      // Same: the list simply does not change.
    }
  }

  return (
    <section>
      <PageHeader
        title={t("nav.playing")}
        detail={current ? undefined : t("playing.idle")}
      >
        {current ? (
          <button
            type="button"
            disabled={busy}
            onClick={() => void saveHere(current)}
          >
            {t("bookmarks.save")}
          </button>
        ) : null}
      </PageHeader>

      {current ? (
        <div className="playing">
          <Artwork
            artworkId={current.artwork_hash}
            title={current.title}
            className="cover large"
          />
          <div className="playing-detail">
            <span className="eyebrow">
              {current.album ?? t("common.album")}
            </span>
            <h2>{current.title}</h2>
            <p className="muted">
              {current.artist ?? t("common.unknownArtist")}
            </p>
            <Lyrics trackId={current.id} />
          </div>
        </div>
      ) : (
        <EmptyState message={t("playing.empty")} />
      )}

      {bookmarks?.length ? (
        <article className="collection">
          <header className="collection-header">
            <div>
              <h3>{t("bookmarks.title")}</h3>
              <span className="muted">{t("bookmarks.detail")}</span>
            </div>
          </header>
          <ul className="list bookmark-list">
            {bookmarks.map((bookmark) => (
              <li key={bookmark.song.id}>
                <button
                  type="button"
                  className="link"
                  onClick={() => player.play([bookmark.song], 0)}
                >
                  <strong>{bookmark.song.title}</strong>
                  <small className="muted">
                    {bookmark.song.artist ?? t("common.unknownArtist")}
                  </small>
                </button>
                <span className="muted">
                  {formatDuration(bookmark.position_ms)}
                </span>
                <button
                  type="button"
                  className="danger"
                  onClick={() => void forget(bookmark.song.id)}
                  aria-label={`${t("bookmarks.forget")}: ${bookmark.song.title}`}
                >
                  {t("bookmarks.forget")}
                </button>
              </li>
            ))}
          </ul>
        </article>
      ) : null}
    </section>
  );
}

export function AdminPage() {
  const signedInUser = currentUser();
  const { t } = useI18n();
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
      setNotice(t("admin.initialScan", { id: result.scan_id }));
      setRevision((value) => value + 1);
    } catch {
      setAdminError(t("admin.libraryError"));
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
      setAdminError(t("admin.accountError"));
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
      setAdminError(t("admin.statusError"));
    }
  }

  async function scanLibrary(libraryId: string) {
    setAdminError(null);
    setNotice(null);
    try {
      const result = await startScan(libraryId);
      setNotice(t("admin.scanQueued", { id: result.scan_id }));
    } catch {
      setAdminError(t("admin.scanError"));
    }
  }

  return (
    <section>
      <PageHeader title={t("admin.title")} detail={t("admin.detail")} />
      {notice ? <p className="notice">{notice}</p> : null}
      {adminError ? <p className="error">{adminError}</p> : null}
      <div className="admin-grid">
        <article className="admin-panel">
          <h3>{t("admin.libraries")}</h3>
          <form
            className="stacked-form"
            onSubmit={(event) => void registerLibrary(event)}
          >
            <input
              value={libraryName}
              onChange={(event) => setLibraryName(event.target.value)}
              placeholder={t("admin.libraryName")}
              required
            />
            <input
              value={libraryPath}
              onChange={(event) => setLibraryPath(event.target.value)}
              placeholder={t("admin.libraryPath")}
              required
            />
            <button type="submit" disabled={libraryBusy}>
              {libraryBusy ? t("admin.registering") : t("admin.register")}
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
                  {t("admin.scan")}
                </button>
              </li>
            ))}
          </ul>
        </article>
        <article className="admin-panel">
          <h3>{t("admin.accounts")}</h3>
          <form
            className="stacked-form"
            onSubmit={(event) => void addUser(event)}
          >
            <input
              value={username}
              onChange={(event) => setUsername(event.target.value)}
              placeholder={t("login.username")}
              autoComplete="off"
              required
            />
            <input
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              placeholder={t("admin.webPassword")}
              minLength={12}
              autoComplete="new-password"
              required
            />
            <button type="submit" disabled={userBusy}>
              {userBusy ? t("admin.creating") : t("admin.create")}
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
                    ? t("admin.currentAccount")
                    : undefined
                }
              >
                {user.disabled ? t("admin.enable") : t("admin.disable")}
              </button>
            </header>
            <CredentialForm user={user} />
          </article>
        ))}
      </div>
    </section>
  );
}

function PageHeader({
  title,
  detail,
  children,
}: {
  title: string;
  /** Omitted while the count it would state is not known yet. */
  detail?: string;
  children?: ReactNode;
}) {
  return (
    <header className="page-header">
      <div className="page-heading">
        <h2>{title}</h2>
        {detail ? <p className="muted">{detail}</p> : null}
      </div>
      {children ? <div className="page-controls">{children}</div> : null}
    </header>
  );
}

/**
 * Text filter shared by the browse pages. It narrows what is already loaded
 * rather than asking the server: `collect` has the whole list in memory, and a
 * per-keystroke round trip would buy nothing.
 */
function FilterField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
}) {
  const { t } = useI18n();
  return (
    <label className="control">
      <span>{label}</span>
      <input
        type="search"
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={t("browse.filterPlaceholder")}
        aria-label={label}
      />
    </label>
  );
}

/** Drops case and diacritics, so "bjork" reaches "Björk". */
function fold(value: string) {
  return value
    .normalize("NFD")
    .replace(/\p{Diacritic}/gu, "")
    .toLowerCase();
}

/** The filter as the comparison sees it. Empty means no filter is applied. */
function normalizeFilter(value: string) {
  return fold(value).trim();
}

function matches(haystack: string | null, needle: string) {
  return haystack ? fold(haystack).includes(needle) : false;
}

function EmptyState({ message }: { message: string }) {
  return (
    <div className="empty-state">
      <p>{message}</p>
    </div>
  );
}

export function AlbumPage({ albumId }: { albumId: string }) {
  const { t } = useI18n();
  const { value, error } = useAsync<AlbumDetail>(
    () => getAlbum(albumId),
    [albumId],
  );
  if (!value) return <Loading error={error} />;
  return (
    <section>
      <header className="detail-header">
        <Artwork
          artworkId={value.artwork_hash}
          title={value.title}
          className="large"
        />
        <div>
          <span className="eyebrow">{t("common.album")}</span>
          <h2>{value.title}</h2>
          <p className="muted">
            {value.artist ?? t("common.variousArtists")}
            {value.year ? ` · ${value.year}` : ""} ·{" "}
            {t("common.tracks", { count: value.songs.length })}
          </p>
        </div>
      </header>
      <SongTable songs={value.songs} />
    </section>
  );
}

export function ArtistsPage() {
  const { t } = useI18n();
  const [filter, setFilter] = useState("");
  const { value, error } = useAsync<Artist[]>(listArtists, []);
  const needle = normalizeFilter(filter);
  // `/api/v2/artists` takes no `sort`, unlike `/albums`: the one order the
  // server offers is alphabetical, so this page filters and does not sort.
  const shown = useMemo(
    () =>
      !value || !needle
        ? (value ?? [])
        : value.filter((artist) => matches(artist.name, needle)),
    [value, needle],
  );
  if (!value) return <Loading error={error} />;
  if (value.length === 0)
    return <p className="muted">{t("common.noArtists")}</p>;
  return (
    <section>
      <PageHeader
        title={t("nav.artists")}
        detail={
          needle
            ? t("browse.matches", { count: shown.length, total: value.length })
            : t("artists.detail", { count: value.length })
        }
      >
        <FilterField
          label={t("browse.filterArtists")}
          value={filter}
          onChange={setFilter}
        />
      </PageHeader>
      {needle && shown.length === 0 ? (
        <EmptyState message={t("browse.noMatch")} />
      ) : null}
      <ul className="list artist-list">
        {shown.map((artist) => (
          <li key={artist.id}>
            <Link to="/artists/$artistId" params={{ artistId: artist.id }}>
              <Artwork
                artworkId={artist.artwork_hash}
                title={artist.name}
                className="artist-avatar"
              />
              <strong>{artist.name}</strong>
            </Link>
            <span className="muted">
              {t("common.albums", { count: artist.album_count })}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}

export function ArtistPage({ artistId }: { artistId: string }) {
  const { t } = useI18n();
  const { value, error } = useAsync<ArtistDetail>(
    () => getArtist(artistId),
    [artistId],
  );
  if (!value) return <Loading error={error} />;
  return (
    <section>
      <header className="artist-hero">
        <Artwork
          artworkId={value.artwork_hash}
          title={value.name}
          className="artist-portrait"
        />
        <div>
          <span className="eyebrow">{t("common.artist")}</span>
          <h2>{value.name}</h2>
          <p className="muted">
            {t("artists.libraryCount", { count: value.album_count })}
          </p>
        </div>
      </header>
      <AlbumGrid albums={value.albums} />
    </section>
  );
}

export function SearchPage() {
  const { t } = useI18n();
  const [query, setQuery] = useState("");
  const [submitted, setSubmitted] = useState("");
  const { value, error } = useAsync<SearchResult | null>(
    () => (submitted ? search(submitted) : Promise.resolve(null)),
    [submitted],
  );

  return (
    <section>
      <PageHeader title={t("nav.search")} detail={t("search.detail")} />
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
          placeholder={t("search.placeholder")}
          aria-label={t("search.query")}
        />
        <button type="submit">{t("nav.search")}</button>
      </form>
      {submitted && !value && <Loading error={error} />}
      {value && (
        <>
          {value.artists.length > 0 && (
            <>
              <h3>{t("nav.artists")}</h3>
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
              <h3>{t("nav.albums")}</h3>
              <AlbumGrid albums={value.albums} />
            </>
          )}
          {value.songs.length > 0 && (
            <>
              <h3>{t("search.tracks")}</h3>
              <SongTable songs={value.songs} />
            </>
          )}
          {value.artists.length === 0 &&
            value.albums.length === 0 &&
            value.songs.length === 0 && (
              <p className="muted">{t("search.empty")}</p>
            )}
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
  const { t } = useI18n();
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
      setError(t("authorize.error"));
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
        <h2>{t("authorize.title")}</h2>
        <p className="error">{t("authorize.invalid")}</p>
      </section>
    );
  }

  return (
    <section className="consent">
      <h2>{t("authorize.client", { client: clientId })}</h2>
      <p className="muted">{t("authorize.detail")}</p>
      <p className="muted">{t("authorize.redirect", { url: redirectUri })}</p>
      {error && <p className="error">{error}</p>}
      <div className="consent-actions">
        <button type="button" onClick={() => void approve()} disabled={busy}>
          {busy ? t("authorize.busy") : t("authorize.approve")}
        </button>
        <button type="button" onClick={deny} disabled={busy}>
          {t("common.cancel")}
        </button>
      </div>
    </section>
  );
}

export function NotFoundPage() {
  const { t } = useI18n();
  return (
    <main className="centered">
      <section className="not-found">
        <span className="not-found-code" aria-hidden="true">
          404
        </span>
        <span className="eyebrow">WaveFlow</span>
        <h1>{t("notFound.title")}</h1>
        <p className="muted">{t("notFound.detail")}</p>
        <Link to="/">{t("notFound.back")}</Link>
      </section>
    </main>
  );
}
