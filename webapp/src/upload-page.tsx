import { type ChangeEvent, useEffect, useRef, useState } from "react";

import {
  ApiError,
  commitUpload,
  getUploadSession,
  negotiateUploads,
  putUploadChunk,
  type UploadDecision,
  type UploadSessionState,
} from "./api";
import type { HashRequest, HashResponse } from "./hash-worker";
import { type TranslationKey, useI18n } from "./i18n";
import { mayUploadTo, useLibraryScope } from "./library-scope";
import { PageHeader } from "./pages";
import {
  extensionOf,
  isUploadable,
  nextRange,
  UPLOAD_EXTENSIONS,
} from "./uploads";

type Status =
  | "waiting"
  | "hashing"
  | "negotiating"
  | "uploading"
  | "committing"
  | "done"
  | "present"
  | "refused"
  | "failed";

type Item = {
  key: number;
  file: File;
  /**
   * Captured when the file is picked. The active library can change while the
   * queue runs, and a file must go where it was dropped, not wherever the
   * picker happens to point when its turn comes.
   */
  libraryId: string;
  status: Status;
  /** Of the current step, from 0 to 1. */
  progress: number;
  reason?: TranslationKey;
};

const ACCEPT = UPLOAD_EXTENSIONS.map((extension) => `.${extension}`).join(",");

const DECISION_REASONS: Record<
  Exclude<UploadDecision, "present" | "accepted">,
  TranslationKey
> = {
  unsupported_format: "upload.reason.unsupportedFormat",
  too_large: "upload.reason.tooLarge",
  quota_exceeded: "upload.reason.quotaExceeded",
  library_closed: "upload.reason.libraryClosed",
  too_many_sessions: "upload.reason.tooManySessions",
};

const STATUS_TEXT: Record<
  Exclude<Status, "refused" | "failed">,
  TranslationKey
> = {
  waiting: "upload.waiting",
  hashing: "upload.hashing",
  negotiating: "upload.negotiating",
  uploading: "upload.uploading",
  committing: "upload.committing",
  done: "upload.done",
  present: "upload.present",
};

/**
 * How many times one fragment is retried after a conflict or a network failure,
 * each time resumed from the server's account of the session.
 */
const RESUME_ATTEMPTS = 3;

class CancelledUpload extends Error {}

let worker: Worker | null = null;
let hashRequests = 0;

/** The browser could not fingerprint the file: its worker failed or never loaded. */
class HashingFailed extends Error {}

/**
 * BLAKE3 of the whole file, computed in a worker. One worker for the page,
 * created on first use: files are fingerprinted one at a time, so a worker per
 * file would pay its start-up again for nothing.
 *
 * A worker that fails — its module refused, or an error before it answers —
 * sends no message at all. Listening for messages alone would leave this
 * promise pending for ever, and the one-at-a-time queue stuck behind it. So a
 * failure rejects the request, and the worker is dropped: the next file starts
 * a fresh one instead of waiting on a broken one.
 */
function hashFile(
  file: File,
  onProgress: (read: number) => void,
): Promise<string> {
  worker ??= new Worker(new URL("./hash-worker.ts", import.meta.url), {
    type: "module",
  });
  const hasher = worker;
  const id = hashRequests++;
  return new Promise((resolve, reject) => {
    const settle = () => {
      hasher.removeEventListener("message", listen);
      hasher.removeEventListener("error", fail);
      hasher.removeEventListener("messageerror", fail);
    };
    const listen = ({ data }: MessageEvent<HashResponse>) => {
      if (data.id !== id) return;
      if ("read" in data) {
        onProgress(data.read);
        return;
      }
      settle();
      if ("hash" in data) resolve(data.hash);
      else reject(new HashingFailed(data.error));
    };
    const fail = () => {
      settle();
      hasher.terminate();
      if (worker === hasher) worker = null;
      reject(new HashingFailed("the fingerprinting worker failed"));
    };
    hasher.addEventListener("message", listen);
    hasher.addEventListener("error", fail);
    hasher.addEventListener("messageerror", fail);
    const request: HashRequest = { id, file };
    hasher.postMessage(request);
  });
}

/**
 * Sends the fragments the session still wants, in order.
 *
 * Only the server's account of the session is ever resumed from. A conflict
 * means the session moved without this call — an acknowledgement lost after
 * the write, or a second tab — and a network failure leaves the last write
 * unknown. Either way the next range is read back from the server rather than
 * guessed.
 */
async function sendFragments(
  file: File,
  session: UploadSessionState,
  onProgress: (sent: number) => void,
  cancelled: () => boolean,
): Promise<void> {
  let state = session;
  let attempts = 0;
  for (
    let range = nextRange(state, file.size);
    range;
    range = nextRange(state, file.size)
  ) {
    if (cancelled()) throw new CancelledUpload();
    try {
      state = await putUploadChunk(
        state.session_id,
        range.index,
        file.slice(range.start, range.end),
      );
      attempts = 0;
      onProgress(state.received_bytes);
    } catch (cause) {
      const resumable =
        (cause instanceof ApiError && cause.status === 409) ||
        cause instanceof TypeError;
      if (!resumable || attempts >= RESUME_ATTEMPTS) throw cause;
      attempts += 1;
      state = await getUploadSession(state.session_id);
      onProgress(state.received_bytes);
    }
  }
}

function failureReason(cause: unknown): TranslationKey {
  if (cause instanceof HashingFailed) return "upload.reason.hashing";
  if (cause instanceof TypeError) return "upload.reason.network";
  if (!(cause instanceof ApiError)) return "upload.reason.error";
  switch (cause.status) {
    case 404:
      return "upload.reason.notAllowed";
    case 422:
      return "upload.reason.refusedFile";
    case 503:
      return "upload.reason.unavailable";
    default:
      return "upload.reason.error";
  }
}

/**
 * Adds files to the active library.
 *
 * One file at a time, through the three steps RFC-008 describes: fingerprint,
 * negotiate, transfer. Fingerprinting first is what lets the server answer
 * `present` before a byte moves, and what makes a file offered again resume the
 * session it left open instead of starting over. One at a time is also what the
 * per-account session limit expects.
 */
export function UploadPage() {
  const { t } = useI18n();
  const { active } = useLibraryScope();
  const [items, setItems] = useState<Item[]>([]);
  const pending = useRef<Item[]>([]);
  const running = useRef(false);
  const cancelled = useRef(false);
  const keys = useRef(0);

  useEffect(() => {
    cancelled.current = false;
    return () => {
      // The session stays open on the server; offering the file again resumes
      // it from where this page stopped.
      cancelled.current = true;
    };
  }, []);

  function update(key: number, change: Partial<Item>) {
    setItems((current) =>
      current.map((item) => (item.key === key ? { ...item, ...change } : item)),
    );
  }

  async function transfer(item: Item) {
    const { key, file, libraryId } = item;
    const fraction = (bytes: number) =>
      file.size === 0 ? 1 : bytes / file.size;
    if (!isUploadable(file.name)) {
      update(key, {
        status: "refused",
        reason: "upload.reason.unsupportedFormat",
      });
      return;
    }
    try {
      update(key, { status: "hashing", progress: 0 });
      const hash = await hashFile(file, (read) =>
        update(key, { progress: fraction(read) }),
      );
      if (cancelled.current) return;
      update(key, { status: "negotiating", progress: 0 });
      const [verdict] = await negotiateUploads(libraryId, [
        {
          full_hash: hash,
          size_bytes: file.size,
          extension: extensionOf(file.name),
        },
      ]);
      if (verdict?.decision === "present") {
        update(key, { status: "present", progress: 1 });
        return;
      }
      if (verdict?.decision !== "accepted" || !verdict.session) {
        update(key, {
          status: "refused",
          reason: verdict
            ? DECISION_REASONS[
                verdict.decision as keyof typeof DECISION_REASONS
              ]
            : "upload.reason.error",
        });
        return;
      }
      const session = verdict.session;
      update(key, {
        status: "uploading",
        progress: fraction(session.received_bytes),
      });
      await sendFragments(
        file,
        session,
        (sent) => update(key, { progress: fraction(sent) }),
        () => cancelled.current,
      );
      update(key, { status: "committing", progress: 1 });
      await commitUpload(session.session_id);
      update(key, { status: "done", progress: 1 });
    } catch (cause) {
      if (cause instanceof CancelledUpload) return;
      update(key, { status: "failed", reason: failureReason(cause) });
    }
  }

  async function run() {
    if (running.current) return;
    running.current = true;
    try {
      for (
        let item = pending.current.shift();
        item;
        item = pending.current.shift()
      ) {
        if (cancelled.current) return;
        await transfer(item);
      }
    } finally {
      running.current = false;
    }
  }

  function pick(event: ChangeEvent<HTMLInputElement>) {
    const files = Array.from(event.target.files ?? []);
    // Cleared, so that picking the same file again after a failure is a change.
    event.target.value = "";
    if (!active || files.length === 0) return;
    const added = files.map(
      (file): Item => ({
        key: keys.current++,
        file,
        libraryId: active.id,
        status: "waiting",
        progress: 0,
      }),
    );
    setItems((current) => [...current, ...added]);
    pending.current.push(...added);
    void run();
  }

  function statusText(item: Item): string {
    if (item.status === "refused" || item.status === "failed") {
      return t(item.status === "refused" ? "upload.refused" : "upload.failed", {
        reason: t(item.reason ?? "upload.reason.error"),
      });
    }
    return t(STATUS_TEXT[item.status]);
  }

  if (!active) {
    return (
      <section className="upload-page">
        <PageHeader title={t("nav.upload")} />
        <p className="notice">{t("upload.noLibrary")}</p>
      </section>
    );
  }

  if (!mayUploadTo(active)) {
    // Two different refusals, and the screen says which. A closed library is
    // the operator's decision and no role changes it; a listener in an open one
    // is a matter of role.
    const mayWrite = active.role === "owner" || active.role === "manager";
    return (
      <section className="upload-page">
        <PageHeader title={t("upload.title", { library: active.name })} />
        <p className="notice">
          {t(mayWrite ? "upload.closed" : "upload.notAllowed")}
        </p>
      </section>
    );
  }

  return (
    <section className="upload-page">
      <PageHeader title={t("upload.title", { library: active.name })} />
      <p className="muted upload-detail">{t("upload.detail")}</p>
      <label className="upload-pick">
        <span>{t("upload.pick")}</span>
        <input type="file" multiple accept={ACCEPT} onChange={pick} />
      </label>
      <p className="muted upload-formats">
        {t("upload.formats", { extensions: UPLOAD_EXTENSIONS.join(", ") })}
      </p>
      {items.length > 0 ? (
        <ul className="list upload-list" aria-live="polite">
          {items.map((item) => (
            <li key={item.key} className={`upload-item ${item.status}`}>
              <span className="upload-name">{item.file.name}</span>
              <span className="upload-status">{statusText(item)}</span>
              <progress
                max={1}
                value={item.progress}
                aria-label={t("upload.progress", { name: item.file.name })}
              />
            </li>
          ))}
        </ul>
      ) : null}
    </section>
  );
}
