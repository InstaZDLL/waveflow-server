/**
 * The frame a page sits in: its header, its wait, its failure, and the way it
 * says something changed.
 *
 * Apart from `pages.tsx` because four screens that are not pages — the tag
 * editor, the canvas panel, the upload screen and the scrobbling settings —
 * imported that module for exactly these, and `pages.tsx` imports
 * `usePlayer`. So each of them reached the player, the queue and every browse
 * screen through a header component.
 *
 * **This is a graph change, not a speed one.** The bundle is one chunk, so
 * moving four symbols out of it moves nothing: what shrinks is what each of
 * those four modules depends on, which is what makes them testable and
 * movable on their own. Splitting by route is the change that would save
 * transfer, and it would save about 4% — React and TanStack are 59% of the
 * bundle and arrive whatever the page — in exchange for a wait on the first
 * visit to every route. That trade is not worth making, and saying so is the
 * point of this paragraph.
 */

import { useQueryClient } from "@tanstack/react-query";
import type { ReactNode } from "react";

import { useI18n } from "./i18n";

/** Eight placeholder cards, keyed by name because their order is their only identity. */
const SKELETON_KEYS = [
  "one",
  "two",
  "three",
  "four",
  "five",
  "six",
  "seven",
  "eight",
];

/**
 * Ask again for what a mutation has just made wrong.
 *
 * Every screen used to hold a `revision` counter and put it in a dependency
 * list, so that bumping it re-ran the load. That worked, and it re-ran
 * exactly one screen: a playlist created here and a playlist listed elsewhere
 * were two loads of the same thing with no idea of each other. Invalidating by
 * key reaches every reader of that answer, which is what "it changed"
 * actually means.
 */
export function useReload(queryKey: readonly unknown[]) {
  const client = useQueryClient();
  return () => {
    void client.invalidateQueries({ queryKey });
  };
}

/**
 * A wait with nothing shaped to promise. The skeleton below stands in for a
 * grid of covers, which would be a lie on a page that has none.
 */
export function Waiting() {
  const { t } = useI18n();
  return (
    <p className="muted" role="status" aria-busy="true">
      {t("common.loading")}
    </p>
  );
}

/**
 * Takes whatever a failed load threw.
 *
 * `unknown` rather than `string`, so a caller hands over what it was given
 * instead of unwrapping it first. A query rejects with an `Error`; the hook
 * this replaced narrowed to a message before the screen ever saw it, and every
 * call site would otherwise have had to repeat that narrowing.
 */
export function Loading({ error }: { error: unknown }) {
  const { t } = useI18n();
  if (error) {
    const detail =
      error instanceof Error
        ? error.message
        : typeof error === "string"
          ? error
          : null;
    return (
      <div className="error-state" role="alert">
        <strong>{t("common.loadError")}</strong>
        <p>{detail}</p>
      </div>
    );
  }
  return (
    <div
      className="skeleton-grid"
      role="status"
      aria-label={t("common.loading")}
      aria-busy="true"
    >
      {SKELETON_KEYS.map((key) => (
        <div className="skeleton-card" key={key}>
          <span />
          <i />
          <i />
        </div>
      ))}
    </div>
  );
}

export function PageHeader({
  title,
  detail,
  children,
}: {
  title: string;
  /** Omitted while the count it would state is not known yet. */
  detail?: string;
  children?: ReactNode;
}) {
  return (
    <header className="page-header">
      <div className="page-heading">
        <h2>{title}</h2>
        {detail ? <p className="muted">{detail}</p> : null}
      </div>
      {children ? <div className="page-controls">{children}</div> : null}
    </header>
  );
}
