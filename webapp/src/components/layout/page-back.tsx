import { useNavigate, useRouter } from "@tanstack/react-router";

import { useI18n } from "../../state/i18n";
import { Icon } from "./icons";

type Navigate = ReturnType<typeof useNavigate>;

/**
 * Back to wherever the page was opened from, above its header.
 *
 * The router's own history, not the browser's: after a full load from a typed
 * address the router has nothing behind it, while the browser may still hold
 * a page of another visit — going there would leave the place the visitor
 * meant to come back from. With nowhere to go back to, `fallback` names the
 * nearest place that makes sense for the page.
 */
export function PageBack({
  fallback,
}: {
  fallback: (navigate: Navigate) => Promise<void>;
}) {
  const { t } = useI18n();
  const router = useRouter();
  const navigate = useNavigate();
  return (
    <button
      type="button"
      className="page-back"
      onClick={() =>
        router.history.canGoBack()
          ? router.history.back()
          : void fallback(navigate)
      }
    >
      <Icon name="back" size={18} />
      {t("common.back")}
    </button>
  );
}
