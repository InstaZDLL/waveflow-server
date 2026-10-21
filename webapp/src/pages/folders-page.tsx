import { useInfiniteQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useMemo } from "react";

import { type DiskFolder, RECEIVED_FOLDER } from "../api";
import { diskTreeQuery } from "../api/queries";
import { Icon } from "../components/icons";
import { Loading, PageHeader } from "../components/page-shell";
import { SongTable } from "../components/song-table";
import { usePlayer } from "../player/player";
import { useI18n } from "../state/i18n";
import { mayUploadTo, useLibraryScope } from "../state/library-scope";

/** The folder's place in the tree, from the root down, each with its path. */
function crumbsOf(path: string): Array<{ name: string; path: string }> {
  if (!path) return [];
  const segments = path.split("/");
  return segments.map((name, index) => ({
    name,
    path: segments.slice(0, index + 1).join("/"),
  }));
}

/**
 * The active library as its files lie on disk (RFC-012 decision 1).
 *
 * Read-only, and said so: the tree is the indexed paths grouped, so it has no
 * "new folder" and no "move" — those would change the owner's file system.
 * The folder RFC-008 receives files into is drawn as **Received**; its path
 * stays the real one, which is what the address carries. Upload leads to
 * RFC-008's own screen, which decides where a received file goes.
 *
 * The folder is in the address (`?path=`), so back and forward walk the tree
 * and a folder can be linked to.
 */
export function FoldersPage({ path }: { path: string }) {
  const { t } = useI18n();
  const player = usePlayer();
  const { active, ready } = useLibraryScope();
  const {
    data,
    error,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
    isFetchNextPageError,
  } = useInfiniteQuery({
    ...diskTreeQuery(active?.id ?? "", path),
    enabled: active !== null,
  });
  const folders = useMemo(
    () => data?.pages.flatMap((page) => page.folders) ?? [],
    [data],
  );
  const tracks = useMemo(
    () => data?.pages.flatMap((page) => page.tracks) ?? [],
    [data],
  );
  const songs = useMemo(() => tracks.map((track) => track.song), [tracks]);
  const names = useMemo(() => tracks.map((track) => track.name), [tracks]);
  const first = data?.pages[0];

  const label = (folder: Pick<DiskFolder, "name" | "path">) =>
    folder.path === RECEIVED_FOLDER ? t("folders.received") : folder.name;
  const crumbs = crumbsOf(path);
  const here = crumbs.at(-1);
  const title = here ? label(here) : t("nav.folders");

  const controls = (
    <>
      {songs.length ? (
        <button
          className="action-primary"
          type="button"
          onClick={() => player.play(songs, 0)}
        >
          <Icon name="play" size={17} />
          {t("songs.playAll", { count: songs.length })}
        </button>
      ) : null}
      {mayUploadTo(active) ? (
        <Link to="/upload" className="folder-upload">
          <Icon name="upload" size={17} />
          {t("folders.upload")}
        </Link>
      ) : null}
    </>
  );

  const header = (
    <PageHeader
      kicker={t("nav.group.library")}
      title={title}
      detail={
        first
          ? [
              t("folders.count", { count: first.folder_total }),
              t("common.tracks", { count: first.track_total }),
            ].join(" · ")
          : undefined
      }
    >
      {controls}
    </PageHeader>
  );

  const breadcrumbs = (
    <nav className="folder-crumbs" aria-label={t("folders.where")}>
      <ol>
        <li>
          <Link
            to="/folders"
            search={{ path: "" }}
            aria-current={crumbs.length === 0 ? "page" : undefined}
          >
            {active?.name ?? t("nav.folders")}
          </Link>
        </li>
        {crumbs.map((crumb, index) => (
          <li key={crumb.path}>
            <Link
              to="/folders"
              search={{ path: crumb.path }}
              aria-current={index === crumbs.length - 1 ? "page" : undefined}
            >
              {label(crumb)}
            </Link>
          </li>
        ))}
      </ol>
    </nav>
  );

  if (ready && !active) {
    return (
      <section>
        {header}
        <p className="muted">{t("folders.noLibrary")}</p>
      </section>
    );
  }

  if (!data) {
    return (
      <section>
        {header}
        {breadcrumbs}
        <Loading error={error} />
      </section>
    );
  }

  return (
    <section>
      {header}
      {breadcrumbs}
      <p className="muted folder-note">{t("folders.readOnly")}</p>
      {folders.length ? (
        <ul className="folder-list" aria-label={t("folders.subfolders")}>
          {folders.map((folder) => (
            <li key={folder.path}>
              <Link
                to="/folders"
                search={{ path: folder.path }}
                className="folder-row"
              >
                <Icon name="folder" size={20} />
                <span>{label(folder)}</span>
                <small className="muted">
                  {t("common.tracks", { count: folder.nested_track_count })}
                </small>
              </Link>
            </li>
          ))}
        </ul>
      ) : null}
      {songs.length ? <SongTable songs={songs} captions={names} /> : null}
      {!folders.length && !songs.length ? (
        <p className="muted">{t("folders.empty")}</p>
      ) : null}
      {hasNextPage && (
        <div className="actions">
          <button
            type="button"
            onClick={() => fetchNextPage()}
            disabled={isFetchingNextPage}
          >
            {isFetchingNextPage ? t("songs.loading") : t("songs.loadMore")}
          </button>
          {isFetchNextPageError ? (
            <p className="error" role="alert">
              {t("songs.loadMoreError")}
            </p>
          ) : null}
        </div>
      )}
    </section>
  );
}
