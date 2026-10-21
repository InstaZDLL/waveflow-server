import type { Song } from "./catalog";
import { call } from "./client";

/**
 * Filed folders (RFC-012 decision 2): the library sorted in WaveFlow, never
 * on disk. A track sits in at most one; one in none is unfiled.
 */
export type FiledFolder = {
  id: string;
  /** `null` at the root. */
  parent_id: string | null;
  name: string;
  /** The available tracks filed directly in it. */
  track_count: number;
};

export type FiledTree = {
  folders: FiledFolder[];
  unfiled_track_count: number;
};

/** What the tracks route calls the root's unfiled entry. */
export const UNFILED = "unfiled";

/** The most tracks one placement files: the server's cap. */
export const MAX_PLACEMENTS = 500;

/** How many tracks one page of a folder carries. */
export const FOLDER_TRACKS_PAGE = 200;

export const listFolders = (libraryId: string) =>
  call<FiledTree>(`/api/v2/libraries/${libraryId}/folders`);

export const createFolder = (
  libraryId: string,
  name: string,
  parentId: string | null,
) =>
  call<FiledFolder>(`/api/v2/libraries/${libraryId}/folders`, {
    method: "POST",
    body: JSON.stringify({ name, parent_id: parentId }),
  });

/**
 * A rename, a move, or both. `parent_id` left out stays where it is; `null`
 * moves the folder to the root.
 */
export const updateFolder = (
  folderId: string,
  patch: { name?: string; parent_id?: string | null },
) =>
  call<FiledFolder>(`/api/v2/folders/${folderId}`, {
    method: "PATCH",
    body: JSON.stringify(patch),
  });

/** Deletes a folder and its sub-folders; their tracks become unfiled. */
export const deleteFolder = (folderId: string) =>
  call<{ unfiled: number }>(`/api/v2/folders/${folderId}`, {
    method: "DELETE",
  });

/** One page of the tracks filed in a folder, or of the unfiled ones. */
export const listFolderTracks = (
  libraryId: string,
  folder: string,
  offset: number,
  limit = FOLDER_TRACKS_PAGE,
  signal?: AbortSignal,
) =>
  call<Song[]>(
    `/api/v2/libraries/${libraryId}/folders/${encodeURIComponent(folder)}/tracks?${new URLSearchParams(
      { offset: String(offset), limit: String(limit) },
    )}`,
    { signal },
  );

/**
 * Files tracks into one folder, or unfiles them with `null`. All or nothing
 * on the server; more than the cap is sent as several requests.
 */
export async function placeTracks(
  libraryId: string,
  trackIds: string[],
  folderId: string | null,
): Promise<number> {
  let placed = 0;
  for (let start = 0; start < trackIds.length; start += MAX_PLACEMENTS) {
    const answer = await call<{ placed: number }>(
      `/api/v2/libraries/${libraryId}/placements`,
      {
        method: "PUT",
        body: JSON.stringify({
          track_ids: trackIds.slice(start, start + MAX_PLACEMENTS),
          folder_id: folderId,
        }),
      },
    );
    placed += answer.placed;
  }
  return placed;
}

/**
 * The folders from the root down to `folderId`, itself included; empty when
 * the folder is not in the tree. Walks parents, never more than the tree
 * holds, so a corrupt answer cannot loop it.
 */
export function folderTrail(
  folders: FiledFolder[],
  folderId: string,
): FiledFolder[] {
  const byId = new Map(folders.map((folder) => [folder.id, folder]));
  const trail: FiledFolder[] = [];
  let at = byId.get(folderId);
  while (at && trail.length <= folders.length) {
    trail.unshift(at);
    at = at.parent_id ? byId.get(at.parent_id) : undefined;
  }
  return trail;
}

/**
 * The tree flattened in display order — siblings as the server ordered them,
 * each followed by its own sub-folders — with each folder's depth, the
 * root's children at 0.
 *
 * **No sort here.** The server orders folders by its `natural_order`
 * collation, and the cards beside this list draw them in that order; a second
 * sort written in JavaScript would be a second rule, and two rules drift — the
 * picker and the cards would disagree on where "Ä 2" goes.
 */
export function folderOutline(
  folders: FiledFolder[],
): Array<{ folder: FiledFolder; depth: number }> {
  const children = new Map<string | null, FiledFolder[]>();
  for (const folder of folders) {
    const siblings = children.get(folder.parent_id) ?? [];
    siblings.push(folder);
    children.set(folder.parent_id, siblings);
  }
  const outline: Array<{ folder: FiledFolder; depth: number }> = [];
  const seen = new Set<string>();
  const walk = (parent: string | null, depth: number) => {
    for (const folder of children.get(parent) ?? []) {
      if (seen.has(folder.id)) continue;
      seen.add(folder.id);
      outline.push({ folder, depth });
      walk(folder.id, depth + 1);
    }
  };
  walk(null, 0);
  return outline;
}

/** Whether `candidate` is `folderId` or sits anywhere beneath it. */
export function isWithin(
  folders: FiledFolder[],
  candidate: string,
  folderId: string,
): boolean {
  return folderTrail(folders, candidate).some(
    (folder) => folder.id === folderId,
  );
}
