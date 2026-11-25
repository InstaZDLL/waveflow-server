import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Library } from "../api";
import {
  LibraryScopeProvider,
  RELIST_FAST_MS,
  RELIST_FAST_TIMES,
  RELIST_SLOW_MS,
  useLibraryScope,
} from "./library-scope";

const listLibraries = vi.hoisted(() => vi.fn());
vi.mock("../api", async (actual) => ({
  ...(await actual<typeof import("../api")>()),
  listLibraries,
}));

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}

let container: HTMLDivElement;
let root: Root;
let scope: ReturnType<typeof useLibraryScope> | null = null;

function Probe() {
  scope = useLibraryScope();
  return null;
}

async function mount() {
  await act(async () => {
    root.render(
      <LibraryScopeProvider>
        <Probe />
      </LibraryScopeProvider>,
    );
  });
}

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  localStorage.clear();
  listLibraries.mockReset();
  scope = null;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const music = { id: "lib-1", name: "Music" } as Library;

/**
 * Whether an empty list says "this account reaches no library": the shell
 * shows that message in place of the catalogue, and must not show it for a
 * list it never received.
 */
describe("LibraryScopeProvider", () => {
  it("counts an empty list it received as listed", async () => {
    listLibraries.mockResolvedValue([]);
    await mount();
    expect(scope?.ready).toBe(true);
    expect(scope?.listed).toBe(true);
    expect(scope?.libraries).toEqual([]);
  });

  it("does not count a list it could not have", async () => {
    listLibraries.mockRejectedValue(new Error("offline"));
    await mount();
    // Ready, so the screens stop waiting, but nothing is known of the account.
    expect(scope?.ready).toBe(true);
    expect(scope?.listed).toBe(false);
  });

  it("unsays an empty list when a refresh cannot list again", async () => {
    // The first list was empty; a library is registered; the refresh fails.
    listLibraries
      .mockResolvedValueOnce([])
      .mockRejectedValueOnce(new Error("offline"));
    await mount();
    expect(scope?.listed).toBe(true);

    await act(async () => scope?.refresh());
    expect(scope?.listed).toBe(false);
  });

  it("lists again on refresh, and scopes to a library that appeared", async () => {
    listLibraries.mockResolvedValueOnce([]).mockResolvedValueOnce([music]);
    await mount();
    expect(scope?.active).toBeNull();

    await act(async () => scope?.refresh());
    expect(listLibraries).toHaveBeenCalledTimes(2);
    expect(scope?.libraries).toEqual([music]);
    expect(scope?.active?.id).toBe("lib-1");
  });
});

/**
 * A library can appear while the list is empty — setup makes the first one
 * after its answer on a slow mount — and nothing but the scope asks again.
 */
describe("LibraryScopeProvider asking again", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  // One step per `act`: React renders when an `act` ends, and the next
  // timer is armed by that render.
  async function advance(ms: number, step = RELIST_FAST_MS) {
    for (let done = 0; done < ms; done += step) {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(Math.min(step, ms - done));
      });
    }
  }

  it("asks an empty list again until a library appears, then stops", async () => {
    listLibraries
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([])
      .mockResolvedValue([music]);
    await mount();
    expect(listLibraries).toHaveBeenCalledTimes(1);

    await advance(RELIST_FAST_MS);
    expect(listLibraries).toHaveBeenCalledTimes(2);
    await advance(RELIST_FAST_MS);
    expect(listLibraries).toHaveBeenCalledTimes(3);
    expect(scope?.active?.id).toBe("lib-1");

    await advance(RELIST_SLOW_MS * 3);
    expect(listLibraries).toHaveBeenCalledTimes(3);
  });

  it("asks a list it could not read again", async () => {
    // The screen that says "no library yet" is not drawn while the list
    // cannot be read, so the asking cannot live there.
    listLibraries
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValue([music]);
    await mount();
    expect(scope?.listed).toBe(false);

    await advance(RELIST_FAST_MS);
    expect(scope?.listed).toBe(true);
    expect(scope?.libraries).toEqual([music]);
  });

  it("slows down after two minutes, and never gives up", async () => {
    listLibraries.mockResolvedValue([]);
    await mount();
    await advance(RELIST_FAST_MS * RELIST_FAST_TIMES);
    expect(listLibraries).toHaveBeenCalledTimes(1 + RELIST_FAST_TIMES);

    await advance(RELIST_FAST_MS);
    expect(listLibraries).toHaveBeenCalledTimes(1 + RELIST_FAST_TIMES);
    await advance(RELIST_SLOW_MS - RELIST_FAST_MS);
    expect(listLibraries).toHaveBeenCalledTimes(2 + RELIST_FAST_TIMES);
    await advance(RELIST_SLOW_MS * 10);
    expect(listLibraries).toHaveBeenCalledTimes(12 + RELIST_FAST_TIMES);
  });
});
