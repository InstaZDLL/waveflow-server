// Uploading files into a library: fragments, refusals, who may upload.

import { blake3 } from "@noble/hashes/blake3.js";
import { bytesToHex } from "@noble/hashes/utils.js";
import { expect, test } from "@playwright/test";

import { expectNoWcagViolations } from "./fixtures/accessibility";
import { resetAndMockApi } from "./fixtures/api";
import { library } from "./fixtures/catalogue";
import { mock } from "./fixtures/state";

test.beforeEach(resetAndMockApi);

/**
 * A file goes through the three steps RFC-008 describes, and the wire carries
 * what the server checks: the whole file's BLAKE3 as the offer, fragments of
 * exactly the advertised size in order, and one commit.
 *
 * The acknowledgement of the second fragment is lost on purpose. The client
 * must read the session back and carry on from where the server stands: not
 * send the written fragment again, and not skip the next one.
 */
test("uploads a file in fragments, resuming from the server's account", async ({
  page,
}) => {
  mock.libraries = [
    { ...library("library-1", "Ma musique"), accepts_uploads: true },
  ];
  mock.loseAcknowledgementOf = 1;
  const bytes = Buffer.from(Array.from({ length: 20 }, (_, i) => i));

  await page.goto("/upload");
  await expect(
    page.getByRole("heading", { name: "Upload to Ma musique" }),
  ).toBeVisible();
  await page.getByLabel("Choose audio files").setInputFiles({
    name: "Army of Me.flac",
    mimeType: "audio/flac",
    buffer: bytes,
  });
  await expect(page.getByText("Added to the library")).toBeVisible();

  // The browser's digest is the one the server recomputes, computed here by an
  // independent call rather than read back from the page.
  expect(mock.uploadOffers).toEqual([
    { full_hash: bytesToHex(blake3(bytes)), size_bytes: 20, extension: "flac" },
  ]);
  expect(
    mock.uploadChunks.map(({ index, bytes: sent }) => [index, sent.length]),
  ).toEqual([
    [0, 8],
    [1, 8],
    [2, 4],
  ]);
  expect(mock.uploadChunks.flatMap(({ bytes: sent }) => sent)).toEqual([
    ...bytes,
  ]);
  expect(mock.uploadCommits).toEqual(["upload-1"]);

  await expectNoWcagViolations(page);
});

/**
 * An extension the scanner does not index is refused on the spot. Hashing it
 * first would spend the time of a full read to be told `unsupported_format`.
 */
test("refuses a file the library cannot index, before hashing it", async ({
  page,
}) => {
  mock.libraries = [
    { ...library("library-1", "Ma musique"), accepts_uploads: true },
  ];
  await page.goto("/upload");
  await page.getByLabel("Choose audio files").setInputFiles({
    name: "notes.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("not audio"),
  });
  await expect(
    page.getByText("Refused: not an audio format the library can index"),
  ).toBeVisible();
  expect(mock.uploadOffers).toEqual([]);
});

/**
 * A worker that cannot load sends no message at all. Waiting for one would leave
 * the row on "Fingerprinting…" for ever, and the queue — one file at a time —
 * stuck behind it. Both files must fail, the second without waiting on the
 * first, and nothing may be offered to the server.
 */
test("fails a file whose fingerprinting worker cannot load, and moves on", async ({
  page,
}) => {
  mock.libraries = [
    { ...library("library-1", "Ma musique"), accepts_uploads: true },
  ];
  await page.route("**/assets/hash-worker-*.js", (route) =>
    route.fulfill({ status: 404, body: "" }),
  );
  await page.goto("/upload");
  await page.getByLabel("Choose audio files").setInputFiles([
    {
      name: "first.flac",
      mimeType: "audio/flac",
      buffer: Buffer.from([1, 2, 3]),
    },
    {
      name: "second.flac",
      mimeType: "audio/flac",
      buffer: Buffer.from([4, 5, 6]),
    },
  ]);
  await expect(
    page.getByText("Failed: this browser could not fingerprint the file"),
  ).toHaveCount(2);
  expect(mock.uploadOffers).toEqual([]);
});

/**
 * Two locks, and the link needs both: a role that may upload, and a library
 * its operator has opened. The page behind it says which one is missing when
 * reached directly.
 */
test("offers the upload only where the library takes files and the role may add them", async ({
  page,
}) => {
  // The link is a section of Settings now, and the gate moved with it. The
  // section menu is drawn at every width, so a zero below is a zero on a
  // phone as well — the rail entry it replaces was counted in the DOM because
  // the rail is hidden there.
  const link = page.locator('.section-menu a[href="/settings/upload"]');
  const account = page.getByRole("heading", { name: "Account", exact: true });

  // The fixture's default: an owner, in a library closed to files.
  await page.goto("/settings");
  await expect(account).toBeVisible();
  await expect(link).toHaveCount(0);

  mock.libraries = [
    { ...library("library-1", "Ma musique"), accepts_uploads: true },
  ];
  await page.reload();
  await expect(account).toBeVisible();
  await expect(link).toHaveCount(1);

  mock.libraries = [
    {
      ...library("library-1", "Ma musique"),
      accepts_uploads: true,
      role: "listener",
    },
  ];
  await page.reload();
  await expect(account).toBeVisible();
  await expect(link).toHaveCount(0);
  await page.goto("/upload");
  await expect(
    page.getByText(
      "Only an owner or a manager of this library can add files to it.",
    ),
  ).toBeVisible();
});
