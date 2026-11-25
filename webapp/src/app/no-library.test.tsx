import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { I18nProvider } from "../state/i18n";
import { NO_LIBRARY_POLL_MS, NO_LIBRARY_POLLS, NoLibrary } from "./no-library";

const refresh = vi.hoisted(() => vi.fn());
vi.mock("../state/library-scope", () => ({
  useLibraryScope: () => ({ refresh }),
}));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  vi.useFakeTimers();
  refresh.mockReset();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

/**
 * A library can appear while this shows — setup makes the first one after its
 * answer on a slow mount — and nothing else would ask for the list again.
 */
describe("NoLibrary", () => {
  it("asks for the list again while it shows, and then stops", () => {
    act(() =>
      root.render(
        <I18nProvider>
          <NoLibrary />
        </I18nProvider>,
      ),
    );
    expect(refresh).not.toHaveBeenCalled();

    act(() => vi.advanceTimersByTime(NO_LIBRARY_POLL_MS));
    expect(refresh).toHaveBeenCalledTimes(1);

    act(() =>
      vi.advanceTimersByTime(NO_LIBRARY_POLL_MS * NO_LIBRARY_POLLS * 2),
    );
    expect(refresh).toHaveBeenCalledTimes(NO_LIBRARY_POLLS);
  });

  it("stops asking once it is gone", () => {
    act(() =>
      root.render(
        <I18nProvider>
          <NoLibrary />
        </I18nProvider>,
      ),
    );
    act(() => root.render(<I18nProvider>{null}</I18nProvider>));
    act(() => vi.advanceTimersByTime(NO_LIBRARY_POLL_MS * 3));
    expect(refresh).not.toHaveBeenCalled();
  });
});
