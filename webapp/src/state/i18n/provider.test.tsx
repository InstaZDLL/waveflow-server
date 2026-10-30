// The provider's starting language, read from the browser on first visit.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { I18nProvider, useI18n } from ".";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}

function Probe() {
  const { locale, t } = useI18n();
  return (
    <p>
      {locale} {t("nav.settings")}
    </p>
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

  it("opens in Simplified Chinese for a browser set to it", () => {
    mountWithBrowserLanguage("zh-CN");
    expect(container.textContent).toBe("zh-CN 设置");
    expect(document.documentElement.lang).toBe("zh-CN");
  });

  it("keeps the language saved on an earlier visit", () => {
    localStorage.setItem("waveflow.locale", "fr");
    mountWithBrowserLanguage("zh-CN");
    expect(container.textContent).toBe("fr Réglages");
  });
});
