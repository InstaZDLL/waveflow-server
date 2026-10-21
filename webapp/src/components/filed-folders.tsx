import {
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import {
  type DragEvent,
  type FormEvent,
  type ReactNode,
  useMemo,
  useState,
} from "react";

import {
  ApiError,
  createFolder,
  deleteFolder,
  type FiledFolder,
  folderTrail,
  isWithin,
  placeTracks,
  UNFILED,
  updateFolder,
} from "../api";
import { filedTreeQuery, folderTracksQuery } from "../api/queries";
import { usePlayer } from "../player/player";
import { useI18n } from "../state/i18n";
import {
  mayCorrectTracks,
  mayUploadTo,
  useLibraryScope,
} from "../state/library-scope";
import { FolderPicker } from "./folder-picker";
import { Icon } from "./icons";
import { Loading, PageHeader } from "./page-shell";
import { SongTable, TRACK_DRAG_TYPE } from "./song-table";

/** What a dragged folder card carries: its id. */
const FOLDER_DRAG_TYPE = "application/x-waveflow-folder";

type Notice = { tone: "status" | "error"; text: string } | null;

/**
 * The library as its owner and managers sort it (RFC-012 decision 2): folders
 * that exist only in WaveFlow, a track in at most one of them.
 *
 * `folder` is empty for the root, `unfiled` for the tracks filed nowhere, and
 * otherwise a folder id. Any member walks it; owners and managers also create,
 * rename, move and delete folders, and file tracks — from the track menu, by
 * dragging a row onto a folder, or by dragging a folder onto another. Every
 * drag has a keyboard twin: the menu's "Move to folder…" and the folder's own
 * "Move…".
 */
export function FiledFolders({
  folder,
  switcher,
}: {
  folder: string;
  switcher: ReactNode;
}) {
  const { t } = useI18n();
  const player = usePlayer();
  const navigate = useNavigate();
  const client = useQueryClient();
  const { active, ready } = useLibraryScope();
  const libraryId = active?.id ?? "";
  const may = mayCorrectTracks(active ?? undefined);
  const tree = useQuery({
    ...filedTreeQuery(libraryId),
    enabled: active !== null,
  });
  const listsTracks = folder !== "";
  const tracks = useInfiniteQuery({
    ...folderTracksQuery(libraryId, folder || UNFILED),
    enabled: active !== null && listsTracks,
  });
  // A disabled query still answers from its cache: at the root, the unfiled
  // tracks read on the way in would otherwise be offered to play and to page.
  const songs = useMemo(
    () => (listsTracks ? (tracks.data?.pages.flat() ?? []) : []),
    [listsTracks, tracks.data],
  );
  const morePages = listsTracks && tracks.hasNextPage;
  const [notice, setNotice] = useState<Notice>(null);
  const [form, setForm] = useState<"create" | "rename" | null>(null);
  const [moving, setMoving] = useState(false);
  const [dropTarget, setDropTarget] = useState<string | null>(null);

  const folders = tree.data?.folders ?? [];
  const isReal = folder !== "" && folder !== UNFILED;
  const trail = isReal ? folderTrail(folders, folder) : [];
  const here = trail.at(-1);
  const missing = isReal && tree.data !== undefined && !here;
  // A folder's sub-folders; at the root and beside the unfiled tracks, the
  // root's own folders — there, the places an unfiled track can be dropped.
  const children = folders.filter(
    (entry) => entry.parent_id === (isReal ? folder : null),
  );
  const title =
    folder === UNFILED ? t("filed.unfiled") : (here?.name ?? t("filed.title"));

  async function refresh() {
    await Promise.all([
      client.invalidateQueries({ queryKey: ["filed-tree", libraryId] }),
      client.invalidateQueries({ queryKey: ["folder-tracks", libraryId] }),
    ]);
  }

  /** Runs a write, says how it went, and re-reads what it changed. */
  async function attempt(
    write: () => Promise<string>,
    refused?: (error: ApiError) => string | null,
  ) {
    setNotice(null);
    try {
      const said = await write();
      await refresh();
      setNotice({ tone: "status", text: said });
      return true;
    } catch (error) {
      const text =
        (error instanceof ApiError && refused?.(error)) || t("filed.failed");
      setNotice({ tone: "error", text });
      return false;
    }
  }

  const named = (error: ApiError) =>
    error.status === 409
      ? t("filed.nameTaken")
      : error.status === 422
        ? t("filed.refused")
        : null;

  function fileInto(target: FiledFolder | null, trackIds: string[]) {
    void attempt(async () => {
      const count = await placeTracks(libraryId, trackIds, target?.id ?? null);
      return t("filed.filed", {
        count,
        folder: target?.name ?? t("filed.unfiled"),
      });
    });
  }

  function moveFolder(moved: string, target: string | null) {
    const name = folders.find((entry) => entry.id === moved)?.name ?? "";
    void attempt(async () => {
      await updateFolder(moved, { parent_id: target });
      return t("filed.folderMoved", { name });
    }, named);
  }

  // A drop target accepts a row, or a folder that is not itself or above it.
  const dropProps = (target: FiledFolder | null) => {
    if (!may) return {};
    const key = target?.id ?? UNFILED;
    const accepts = (event: DragEvent) =>
      event.dataTransfer.types.includes(TRACK_DRAG_TYPE) ||
      (target !== null && event.dataTransfer.types.includes(FOLDER_DRAG_TYPE));
    return {
      onDragOver: (event: DragEvent) => {
        if (!accepts(event)) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = "move";
        setDropTarget(key);
      },
      onDragLeave: () =>
        setDropTarget((current) => (current === key ? null : current)),
      onDrop: (event: DragEvent) => {
        setDropTarget(null);
        const rows = event.dataTransfer.getData(TRACK_DRAG_TYPE);
        const dragged = event.dataTransfer.getData(FOLDER_DRAG_TYPE);
        if (rows) {
          event.preventDefault();
          fileInto(target, JSON.parse(rows) as string[]);
        } else if (
          dragged &&
          target &&
          !isWithin(folders, target.id, dragged)
        ) {
          event.preventDefault();
          moveFolder(dragged, target.id);
        }
      },
    };
  };

  async function submitName(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const name = String(new FormData(event.currentTarget).get("name") ?? "");
    const done = await attempt(async () => {
      if (form === "rename" && here) {
        const renamed = await updateFolder(here.id, { name });
        return t("filed.renamed", { name: renamed.name });
      }
      const created = await createFolder(
        libraryId,
        name,
        isReal ? folder : null,
      );
      return t("filed.created", { name: created.name });
    }, named);
    if (done) setForm(null);
  }

  async function remove() {
    if (!here || !window.confirm(t("filed.confirmDelete", { name: here.name })))
      return;
    const parent = here.parent_id;
    const done = await attempt(async () => {
      const { unfiled } = await deleteFolder(here.id);
      return t("filed.deleted", { count: unfiled });
    });
    if (done) {
      await navigate({
        to: "/folders",
        search: { view: "filed", folder: parent ?? "" },
      });
    }
  }

  const link = (target: string) => ({
    to: "/folders" as const,
    search: { view: "filed" as const, folder: target },
  });

  const controls = (
    <>
      {songs.length ? (
        <button
          className="action-primary"
          type="button"
          onClick={() => player.play(songs, 0)}
        >
          <Icon name="play" size={17} />
          {morePages
            ? t("songs.playLoaded", { count: songs.length })
            : t("songs.playAll", { count: songs.length })}
        </button>
      ) : null}
      {may && folder !== UNFILED && !missing ? (
        <button
          type="button"
          onClick={() => setForm(form === "create" ? null : "create")}
        >
          {t("filed.newFolder")}
        </button>
      ) : null}
      {may && here ? (
        <>
          <button
            type="button"
            onClick={() => setForm(form === "rename" ? null : "rename")}
          >
            {t("filed.rename")}
          </button>
          <button type="button" onClick={() => setMoving(true)}>
            {t("filed.move")}
          </button>
          <button
            type="button"
            className="danger"
            onClick={() => void remove()}
          >
            {t("filed.delete")}
          </button>
        </>
      ) : null}
      {here && mayUploadTo(active) ? (
        <Link
          to="/upload"
          search={{ folder: here.id }}
          className="folder-upload"
        >
          <Icon name="upload" size={17} />
          {t("filed.uploadHere")}
        </Link>
      ) : null}
    </>
  );

  const header = (
    <PageHeader
      kicker={t("nav.group.library")}
      title={title}
      detail={
        tree.data && !listsTracks
          ? t("folders.count", { count: folders.length })
          : undefined
      }
    >
      {controls}
    </PageHeader>
  );

  const crumbs = (
    <nav className="folder-crumbs" aria-label={t("folders.where")}>
      <ol>
        <li>
          <Link {...link("")} aria-current={folder === "" ? "page" : undefined}>
            {t("filed.title")}
          </Link>
        </li>
        {folder === UNFILED ? (
          <li>
            <Link {...link(UNFILED)} aria-current="page">
              {t("filed.unfiled")}
            </Link>
          </li>
        ) : null}
        {trail.map((entry, index) => (
          <li key={entry.id}>
            <Link
              {...link(entry.id)}
              aria-current={index === trail.length - 1 ? "page" : undefined}
            >
              {entry.name}
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
        {switcher}
        <p className="muted">{t("folders.noLibrary")}</p>
      </section>
    );
  }

  const card = (
    target: FiledFolder | null,
    label: string,
    count: number,
    to: string,
  ) => (
    <li key={to}>
      <Link
        {...link(to)}
        className={`folder-row${dropTarget === (target?.id ?? UNFILED) ? " drop-target" : ""}`}
        draggable={may && target !== null ? true : undefined}
        onDragStart={
          may && target
            ? (event: DragEvent) => {
                event.dataTransfer.effectAllowed = "move";
                event.dataTransfer.setData(FOLDER_DRAG_TYPE, target.id);
                event.dataTransfer.setData("text/plain", target.name);
              }
            : undefined
        }
        {...dropProps(target)}
      >
        <Icon name="folder" size={20} />
        <span>{label}</span>
        <small className="muted">{t("common.tracks", { count })}</small>
      </Link>
    </li>
  );

  return (
    <section>
      {header}
      {switcher}
      {crumbs}
      <p className="muted folder-note">{t("filed.note")}</p>
      {notice ? (
        <p
          className={notice.tone === "error" ? "error" : "muted folder-notice"}
          role={notice.tone === "error" ? "alert" : "status"}
        >
          {notice.text}
        </p>
      ) : null}
      {form ? (
        <form
          className="folder-form"
          onSubmit={(event) => void submitName(event)}
        >
          <label className="folder-form-field">
            <span>
              {t(form === "rename" ? "filed.renameLabel" : "filed.name")}
            </span>
            <input
              name="name"
              required
              maxLength={120}
              defaultValue={form === "rename" ? here?.name : ""}
              // biome-ignore lint/a11y/noAutofocus: the form opens on request, to be typed into
              autoFocus
            />
          </label>
          <button type="submit" className="action-primary">
            {t(form === "rename" ? "filed.save" : "filed.create")}
          </button>
          <button type="button" onClick={() => setForm(null)}>
            {t("filed.cancel")}
          </button>
        </form>
      ) : null}
      {missing ? (
        <p className="muted">{t("filed.missing")}</p>
      ) : !tree.data ? (
        <Loading error={tree.error} />
      ) : (
        <>
          {(folder !== UNFILED || may) && (children.length || folder === "") ? (
            <ul
              className="folder-list"
              aria-label={t(
                folder === UNFILED ? "filed.fileInto" : "folders.subfolders",
              )}
            >
              {children.map((entry) =>
                card(entry, entry.name, entry.track_count, entry.id),
              )}
              {folder === ""
                ? card(
                    null,
                    t("filed.unfiled"),
                    tree.data.unfiled_track_count,
                    UNFILED,
                  )
                : null}
            </ul>
          ) : null}
          {listsTracks ? (
            !tracks.data ? (
              <Loading error={tracks.error} />
            ) : songs.length ? (
              <SongTable songs={songs} draggableRows={may} />
            ) : !children.length || folder === UNFILED ? (
              <p className="muted">
                {t(folder === UNFILED ? "filed.allFiled" : "filed.empty")}
              </p>
            ) : null
          ) : null}
          {morePages ? (
            <div className="actions">
              <button
                type="button"
                onClick={() => tracks.fetchNextPage()}
                disabled={tracks.isFetchingNextPage}
              >
                {tracks.isFetchingNextPage
                  ? t("songs.loading")
                  : t("songs.loadMore")}
              </button>
              {tracks.isFetchNextPageError ? (
                <p className="error" role="alert">
                  {t("songs.loadMoreError")}
                </p>
              ) : null}
            </div>
          ) : null}
        </>
      )}
      {moving && here ? (
        <FolderPicker
          libraryId={libraryId}
          title={t("filed.moveFolder", { name: here.name })}
          rootLabel={t("filed.topLevel")}
          current={here.parent_id}
          exclude={here.id}
          onPick={async (target) => {
            await updateFolder(here.id, { parent_id: target });
            await refresh();
            setNotice({
              tone: "status",
              text: t("filed.folderMoved", { name: here.name }),
            });
          }}
          onClose={() => setMoving(false)}
        />
      ) : null}
    </section>
  );
}
