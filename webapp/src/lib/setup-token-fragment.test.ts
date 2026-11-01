import { describe, expect, it, vi } from "vitest";

import { takeSetupTokenFromFragment } from "./setup-token-fragment";

/** A location at `pathname + search + hash`, and a history that records. */
function at(hash: string, pathname = "/login", search = "") {
  const location = { hash, pathname, search };
  const state = { __TSR_key: "k" };
  const history = { state, replaceState: vi.fn() };
  return { location, history, state };
}

describe("takeSetupTokenFromFragment", () => {
  it("takes the token and drops it from the address bar", () => {
    const { location, history, state } = at("#setup-token=wfsetup_abc-DEF_123");
    expect(takeSetupTokenFromFragment(location, history)).toBe(
      "wfsetup_abc-DEF_123",
    );
    expect(history.replaceState).toHaveBeenCalledOnce();
    expect(history.replaceState).toHaveBeenCalledWith(state, "", "/login");
  });

  it("decodes what the server escaped", () => {
    // `handover_url` escapes everything but the unreserved characters.
    const { location, history } = at(
      "#setup-token=my%20token%20%26%20100%25%20sure%20%2B%20%C3%A9",
    );
    expect(takeSetupTokenFromFragment(location, history)).toBe(
      "my token & 100% sure + é",
    );
  });

  it("keeps the query and any other fragment parameter", () => {
    const { location, history, state } = at(
      "#a=1&setup-token=wfsetup_x&b=2",
      "/login",
      "?next=%2Falbums",
    );
    expect(takeSetupTokenFromFragment(location, history)).toBe("wfsetup_x");
    expect(history.replaceState).toHaveBeenCalledWith(
      state,
      "",
      "/login?next=%2Falbums#a=1&b=2",
    );
  });

  it("clears an empty token from the address and fills nothing", () => {
    const { location, history } = at("#setup-token=");
    expect(takeSetupTokenFromFragment(location, history)).toBeNull();
    expect(history.replaceState).toHaveBeenCalledWith(
      expect.anything(),
      "",
      "/login",
    );
  });

  it("leaves an address without a token alone", () => {
    for (const hash of ["", "#", "#other=1"]) {
      const { location, history } = at(hash);
      expect(takeSetupTokenFromFragment(location, history)).toBeNull();
      expect(history.replaceState).not.toHaveBeenCalled();
    }
  });

  it("reads the real window by default", () => {
    window.history.replaceState(null, "", "/login#setup-token=wfsetup_real");
    expect(takeSetupTokenFromFragment()).toBe("wfsetup_real");
    expect(window.location.hash).toBe("");
    expect(window.location.pathname).toBe("/login");
  });
});
