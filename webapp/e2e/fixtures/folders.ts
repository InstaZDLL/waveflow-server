// The filed folder tree a library's tracks are served under.

import type { Page } from "@playwright/test";

import type { song } from "./catalogue";

/**
 * A stand-in for the filed-folder routes (RFC-012 decision 2), with state: the
 * screen re-reads after every write, and the point is that it shows what the
 * write did. Names collide case-insensitively, as on the server.
 */
export async function serveFiledFolders(
  page: Page,
  songs: ReturnType<typeof song>[],
) {
  const state = {
    folders: [] as Array<{
      id: string;
      parent_id: string | null;
      name: string;
    }>,
    placed: new Map<string, string>(),
    writes: [] as Array<{ method: string; path: string; body: unknown }>,
  };
  const tree = () => ({
    folders: state.folders.map((folder) => ({
      ...folder,
      track_count: [...state.placed.values()].filter((id) => id === folder.id)
        .length,
    })),
    unfiled_track_count: songs.filter((entry) => !state.placed.has(entry.id))
      .length,
  });
  await page.route("**/api/v2/libraries/library-1/folders", async (route) => {
    const request = route.request();
    if (request.method() === "POST") {
      const body = request.postDataJSON() as {
        name: string;
        parent_id: string | null;
      };
      state.writes.push({ method: "POST", path: "folders", body });
      const name = body.name.trim();
      if (
        state.folders.some(
          (folder) =>
            folder.parent_id === body.parent_id &&
            folder.name.toLowerCase() === name.toLowerCase(),
        )
      ) {
        await route.fulfill({ status: 409, json: { error: "conflict" } });
        return;
      }
      const folder = {
        id: `folder-${state.folders.length + 1}`,
        parent_id: body.parent_id,
        name,
      };
      state.folders.push(folder);
      await route.fulfill({ status: 201, json: { ...folder, track_count: 0 } });
      return;
    }
    await route.fulfill({ json: tree() });
  });
  await page.route(
    "**/api/v2/libraries/library-1/folders/*/tracks?*",
    (route) => {
      const folder = new URL(route.request().url()).pathname.split("/").at(-2);
      return route.fulfill({
        json: songs.filter((entry) =>
          folder === "unfiled"
            ? !state.placed.has(entry.id)
            : state.placed.get(entry.id) === folder,
        ),
      });
    },
  );
  await page.route(
    "**/api/v2/libraries/library-1/placements",
    async (route) => {
      const body = route.request().postDataJSON() as {
        track_ids: string[];
        folder_id: string | null;
      };
      state.writes.push({ method: "PUT", path: "placements", body });
      for (const id of body.track_ids) {
        if (body.folder_id) state.placed.set(id, body.folder_id);
        else state.placed.delete(id);
      }
      await route.fulfill({ json: { placed: body.track_ids.length } });
    },
  );
  await page.route("**/api/v2/folders/*", async (route) => {
    const request = route.request();
    const id = new URL(request.url()).pathname.split("/").at(-1) ?? "";
    if (request.method() === "DELETE") {
      state.writes.push({ method: "DELETE", path: id, body: null });
      const gone = new Set([id]);
      for (const folder of state.folders) {
        if (folder.parent_id && gone.has(folder.parent_id)) gone.add(folder.id);
      }
      let unfiled = 0;
      for (const [track, folder] of [...state.placed]) {
        if (gone.has(folder)) {
          state.placed.delete(track);
          unfiled += 1;
        }
      }
      state.folders = state.folders.filter((folder) => !gone.has(folder.id));
      await route.fulfill({ json: { unfiled } });
      return;
    }
    const patch = request.postDataJSON() as {
      name?: string;
      parent_id?: string | null;
    };
    state.writes.push({ method: "PATCH", path: id, body: patch });
    state.folders = state.folders.map((folder) =>
      folder.id === id
        ? {
            ...folder,
            name: patch.name ?? folder.name,
            parent_id:
              patch.parent_id === undefined
                ? folder.parent_id
                : patch.parent_id,
          }
        : folder,
    );
    const folder = state.folders.find((entry) => entry.id === id);
    await route.fulfill({ json: { ...folder, track_count: 0 } });
  });
  return state;
}
