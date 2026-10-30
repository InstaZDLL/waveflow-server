// The provider's starting language, read from the browser on first visit, and
// fetched before anything is shown in it. Each test asks for a language no
// other test has fetched, since a fetched one stays for the whole file.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { I18nProvider, loadLocale, useI18n } from ".";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}

function Probe() {
  const { locale, setLocale, t } = useI18n();
  return (
    <button type="button" onClick={() => setLocale("de")}>
      {locale} {t("nav.settings")}
    </button>
  );
}

describe("I18nProvider", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;
    localStorage.clear();
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  });

  function mountWithBrowserLanguage(language: string) {
    vi.spyOn(navigator, "language", "get").mockReturnValue(language);
    act(() =>
      root.render(
        <I18nProvider>
          <Probe />
        </I18nProvider>,
      ),
    );
  }

  it("opens in Simplified Chinese for a browser set to it", async () => {
    mountWithBrowserLanguage("zh-CN");
    // Nothing until the strings arrive, rather than English for a moment.
    expect(container.textContent).toBe("");
    await act(() => loadLocale("zh-CN"));
    expect(container.textContent).toBe("zh-CN 设置");
    expect(document.documentElement.lang).toBe("zh-CN");
  });

  it("keeps the language saved on an earlier visit", async () => {
    localStorage.setItem("waveflow.locale", "fr");
    mountWithBrowserLanguage("zh-CN");
    await act(() => loadLocale("fr"));
    expect(container.textContent).toBe("fr Réglages");
  });

  it("keeps the language shown until the chosen one arrives", async () => {
    mountWithBrowserLanguage("en-GB");
    expect(container.textContent).toBe("en Settings");
    act(() => container.querySelector("button")?.click());
    expect(container.textContent).toBe("en Settings");
    await act(() => loadLocale("de"));
    expect(container.textContent).toBe("de Einstellungen");
    expect(localStorage.getItem("waveflow.locale")).toBe("de");
  });
});
