import { Link } from "@tanstack/react-router";
import { useI18n } from "../state/i18n";

export function NotFoundPage() {
  const { t } = useI18n();
  return (
    <main className="centered">
      <section className="not-found">
        <span className="not-found-code" aria-hidden="true">
          404
        </span>
        <span className="eyebrow">WaveFlow</span>
        <h1>{t("notFound.title")}</h1>
        <p className="muted">{t("notFound.detail")}</p>
        <Link to="/">{t("notFound.back")}</Link>
      </section>
    </main>
  );
}
