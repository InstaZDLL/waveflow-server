# RFC-016 — The real quality of a file, measured from its audio

- **Status**: Proposed
- **Implemented by**: [#317](https://github.com/InstaZDLL/waveflow-server/pull/317)
  (this RFC) and [#319](https://github.com/InstaZDLL/waveflow-server/pull/319)
  (steps 2 to 4 together: the migration, the ported detector and the FFmpeg
  pipe, the background analyser, the verdicts, the five routes, the "Needs
  attention" category, the Quality page and the track properties' "Real
  quality" section), then [#320](https://github.com/InstaZDLL/waveflow-server/pull/320)
  (the analysis state in the summary and on the page, and one write per
  measurement). The *Status* field above never flips in this project;
  this line is the one that says what is running.
- **Date**: 2026-10-24
- **Revised**: 2026-10-25, with the implementation. `upsampled` stops at
  30 kHz rather than 24, as measured on FFmpeg's own resampler (decision 3),
  and decision 3 says which upsampled files it cannot see.
- **Revised**: 2026-10-24, after the page ran on a real library and said
  nothing for forty seconds at a time. The summary carries the analyser's
  state (the surface), the page shows it (decision 10), and each
  measurement is written as it finishes rather than sixteen at a time
  (decision 4). A scan the caller cannot see changes nothing they are
  shown, the header and the list refresh together, "Analyse again" is
  offered only at rest, and each row shows its cover (decision 10).
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md) (FFmpeg, read-only
  audio, the path guard, tenancy in the queries),
  [RFC-015](RFC-015-artist-credit-splitting.md) ("Needs attention")
- **Amends**: RFC-015 decision 8, which leaves `probable_fake_lossless` out
  because "the server does not run" a spectral analysis. Decision 9 adds it.

## Problem

Everything the server says about a file's quality today is what the file says
about itself. The scanner reads the container's properties through lofty
(`src/scanner.rs`, `extract_file`): `codec`, `bitrate`, `sample_rate`,
`channels` and `bit_depth`, the last kept only when above zero, so a lossy
codec reports none. The web client grades a file from those five fields
(`webapp/src/components/file-quality.tsx`, `qualityBand`): a bit depth means
lossless, more than 16 bits or more than 48 kHz means hi-res, and below that
the bitrate decides.

None of it is measured. A 128 kbps MP3 decoded and written back as a 16-bit
FLAC is a FLAC: the client lights four of five bars and writes "Lossless".
Written at 24 bits, the same file lights five and writes "Hi-res", and so
does a CD rip resampled to 96 kHz and sold as hi-res. The container is honest
about the container and silent about where the audio came from.

On the instance this was written against, the 35 FLAC files the scanner found
in its one library all report 24 bits at 44.1 kHz, and the client calls every
one of them "Hi-res". Measured from the audio (decision 2), two of them stop
dead at 20.6 and 20.8 kHz, where LAME cuts a 320 kbps MP3, and fourteen of the
35 use 16 bits of their 24. The container cannot tell any of that. The audio
can.

Most lossy encoders low-pass before they encode: LAME cuts a 128 kbps MP3
around 16–17 kHz and a 320 kbps one around 20 kHz, and Opus never codes past
20 kHz. Decoding the result into FLAC keeps the cut. A recording made
lossless has none: its spectrum fades into the noise floor, or runs on to its
converter's own filter just under Nyquist. That cut is visible in the audio,
and it is what this document measures. Its reach is exactly that cut: an
encoder that keeps the full band, as FFmpeg's AAC at 256 kbps and most Vorbis
at q6 do, leaves nothing for it to find (decision 3, and "What is still
open").

## What exists

### On the desktop

The desktop app (`InstaZDLL/WaveFlow`) already finds fake lossless files, and
this document does not invent a second way. Read at the revision the server
pins (`waveflow-core` 1.9.1, `102df003`):

- **The detector** is `waveflow_core::analysis::bandwidth`. It averages the
  power spectrum of the whole file **at the native rate** and **channel by
  channel** (a mono sum cancels highs in opposition between the channels and
  draws a wall a genuine file does not have), smooths it over 100 Hz, and
  searches above 11 kHz for a **wall**: the steepest fall across a few hundred
  hertz, which counts when it is at least 20 dB (`WALL_DB`) and nothing above it
  comes back to within 15 dB of the level below it. A slow fade, however low it
  ends, is not a wall, which is what keeps a dull old recording from being
  taken for an MP3. Frames under −60 dBFS are left out of the average.
- **The verdict** is SQL, computed where it is read
  (`probable_fake_lossless_sql!`, exported by the crate): a lossless format
  (`lossless_track_sql!`: `FLAC`, `WAV`, `AIFF`, `PCM`, `APE`, `WavPack`, or
  ALAC, which the scanner labels `AAC` and which is told apart by having a bit
  depth) whose wall stands at or under `MIN(21000, sample_rate / 2 − 1000)`.
  The 1 kHz margin is for a 32 kHz file, whose converter filter stands near
  15.5 kHz and would otherwise read as a 128 kbps MP3.
- **How well it does**, measured by the desktop on averaged spectra
  (`docs/features/library.md`, "Fake lossless files"): 0% of 41 genuine WAVs
  and 40 full-band FLACs flagged; 95%, 87.5% and 87.5% of fakes made through
  LAME at 128, 192 and 320 kbps caught; 80% through Opus at 160 kbps. It misses
  dark sources whose highs sit so near the floor that the encoder's cut is a
  fall of 12–20 dB, and it cannot see an encoder that does not low-pass:
  FFmpeg's AAC at 256 kbps and most Vorbis at q6 keep the full band.
- **Where it runs**: inside `analysis::analyze_file`, which decodes with
  symphonia and computes, in the same pass, peak, loudness, tempo, beats,
  energy and key. The desktop stores the cutoff in `track_analysis.cutoff_hz`
  (`NULL` for no wall) with `bandwidth_version` beside it, `NULL` until
  measured, so "no wall" and "never measured" stay apart. A detector change
  bumps `BANDWIDTH_VERSION`, and the sweep re-decodes the lossless files alone.
- **Where it shows**: in Track Properties (where the highs stop, and the
  verdict), as the "probable fake lossless" inventory category, and as a smart
  playlist rule. **Not as a badge in track lists**, deliberately: a dull
  recording flagged by mistake would carry the accusation on every row.

The pieces the server can call at the pinned revision are `analyze_file`, the
`Bandwidth` result, the constants `WALL_DB`, `LOSSY_CUTOFF_HZ`,
`NYQUIST_MARGIN_HZ` and `BANDWIDTH_VERSION`, and the two SQL macros. The
streaming meter itself, `BandwidthMeter`, is `pub(crate)`, and so is the
search. Neither the desktop nor the core measures bit depth or detects an
upsampled file, and neither draws a spectrogram.

### On the server

- The scanner stores the five container fields per track, and `full_hash`, a
  BLAKE3 of the whole file, which changes exactly when the audio might have.
- FFmpeg and ffprobe are mandatory since M2 (RFC-002). `src/media.rs` spawns
  FFmpeg for transcodes under a global and a per-user semaphore
  (`WAVEFLOW_TRANSCODE_GLOBAL_LIMIT`, `WAVEFLOW_TRANSCODE_PER_USER_LIMIT`),
  with `kill_on_drop` and a deadline. `src/services/canvas.rs` runs FFmpeg and
  ffprobe under a timeout for the same reasons. `secure_track_path`, private
  to `media.rs`, is the canonical-path and symlink guard a stream goes
  through before a file is opened.
- Background work exists in two shapes: the scanner (at boot, then every
  `WAVEFLOW_SCAN_INTERVAL_SECS`), and RFC-011's enrichment, which is demand
  driven, bounded and never on the answering path.
- `waveflow-core` is built with `default-features = false`, and its symphonia
  is compiled with MP3, FLAC, WAV, AIFF, Vorbis, OGG, AAC, ALAC, MP4 and PCM.
  There is no APE and no WavPack decoder in that set, and no Opus.

## Decision 1 — what is measured, and on which files

Three facts about the audio, each one a claim of the container checked
against what is in it:

| Claim of the container | What is measured | What contradicts it |
| --- | --- | --- |
| lossless | where the highs stop: the desktop's **wall** | a wall where a lossy encoder puts one |
| a sample rate above 48 kHz | the same wall | a wall at a CD or DAT converter's filter, far under the file's Nyquist |
| a bit depth | the **effective bit depth**: how many bits the samples use | fewer bits than the container declares |

**Only lossless files are measured**, as `lossless_track_sql!` defines them,
so the server and the desktop agree on what a lossless file is. That bounds
the cost (decision 4) by what the question needs: a lossy file is already
declaring that it lost something, and its quality is its encoder and its
bitrate, which the scanner already reports. Measuring a lossy file to find a
320 kbps MP3 made from a 128 kbps one is a real question with a different
verdict table, and "What is still open" takes it up. DSD files are not
measured either: their spectrum is shaped noise above 20 kHz by design, and a
wall search means nothing there.

**The effective bit depth** is the number of bits above the lowest one any
sample sets: decode to signed 32-bit integers, OR every sample together, and
it is `32 − trailing_zeros`. A 24-bit FLAC made from a 16-bit source by
padding has its low 8 bits zero in every sample and measures 16. It is exact,
it costs one OR per sample, and it needs no threshold. It is not measured on
a floating-point WAV, whose bits do not mean depth, and a file whose every
sample is zero has none.

## Decision 2 — FFmpeg decodes, and the desktop's detector judges

The decoding and the judging are two separate questions, and they get two
different answers.

**FFmpeg decodes.** The analyser asks ffprobe for the first audio stream's
rate and channel count, then spawns

```text
ffmpeg -nostdin -hide_banner -loglevel error -threads 1
       -i <guarded path> -map 0:a:0 -f s32le -acodec pcm_s32le pipe:1
```

and reads the samples from the pipe, at the file's own rate: nothing is
resampled, since a resampler's filter would draw a wall of its own. The rate
comes from ffprobe and not from the track row, so the meter is never fed at a
rate the decoder did not produce. That is the decoder the server already
depends on and already bounds:

- it reads **every lossless format the scanner indexes**, APE and WavPack
  included, which the server's symphonia cannot;
- it runs **out of process**, so a file that hangs the decoder costs a
  `kill` at the deadline rather than a blocked thread, and a crash costs one
  measurement rather than the server;
- it hands over **integers**, which the bit depth needs, and the meter takes
  them as `f32` divided by 2³¹.

**The desktop's detector judges.** The wall search is the desktop's, with its
constants and its tuning, because it is the measured part and the part this
document has no grounds to change. It lives in the server as a module,
`src/quality/bandwidth.rs`, ported from `waveflow_core::analysis::bandwidth`
at `BANDWIDTH_VERSION` 1, and fed from the pipe instead of from symphonia.
The port is held to the original by two tests rather than by care:

- **the constants**: `WALL_DB`, `LOSSY_CUTOFF_HZ`, `NYQUIST_MARGIN_HZ` and
  `BANDWIDTH_VERSION` are public in the core, and the test asserts the port's
  are equal. A core bump that changes the detector bumps `BANDWIDTH_VERSION`,
  which fails this test until the port follows;
- **parity**: the test writes synthetic WAV fixtures (shaped noise with a cut
  at 16, 19 and 20 kHz, a full band, a gentle fade, highs in opposition
  between the channels: the desktop's own test signals), measures each with
  `waveflow_core::analysis::analyze_file` and with the server's pipeline, and
  asserts the two cutoffs agree within one FFT bin, and that both find or both
  miss a wall.

The core is GPL-3.0-only and the server AGPL-3.0-only. Both are this
project's, by the same author, so the detector's code can be carried into
the server and published under the server's licence, as the rest of the
server is. Were they not, GPLv3 section 13 and its AGPLv3 counterpart are the
terms that permit combining a GPLv3 work with an AGPLv3 one, each part keeping
its own licence.

### Measured before it was proposed

A prototype of the port, fed by the FFmpeg command above, was run beside
`analyze_file` at the pinned revision on the instance's 35 FLAC files and on
nine fakes made from three of them (FFmpeg's `libmp3lame` at 128 and 320 kbps
and `libopus` at 160 kbps, each decoded back into FLAC):

- **Parity**: on all 44 files, the two found the same wall, or the same
  absence of one, with the same cutoff and the same fall to every printed
  digit. The averaging and the search are the same code; only the decoder
  differs, and on lossless input two correct decoders hand over the same
  samples.
- **The instance's files**: two walls at 20.6 and 20.8 kHz (`lossy_source`,
  both also using 16 bits of 24), twelve more files using 16 bits of 24
  (`padded`), and twenty-one `consistent`, seven of them with no wall at all
  and fourteen with a wall at 21.2–21.4 kHz, which is a CD converter's filter
  above the 21 kHz threshold, exactly as the desktop's rule intends.
- **The fakes**: six of nine found, as below.

| Fakes made from three of the files | Found |
| --- | --- |
| LAME 128 kbps | 2 of 3, at 16.4 and 16.7 kHz; the third fell 19.5 dB, just short of 20 |
| LAME 320 kbps | 3 of 3, at 20.0–20.2 kHz |
| Opus 160 kbps | 1 of 3, at 20.2 kHz |

The miss at 128 kbps is the desktop's documented blind spot: the source is a
dark lo-fi track whose own spectrum has no highs to cut. Nine fakes measure
nothing about rates; the desktop's sweep of 81 genuine files and 240 fakes is
the measurement, and this only shows the port inherits it.

### Why not the alternatives

- **Calling `analyze_file`**, the one public entry point, took 12.2 seconds
  for the 35 files (0.35 s each) against 9.4 seconds (0.27 s) for the
  prototype, ffprobe and FFmpeg's process start included. The cost is not the
  argument, at a quarter less. What decides is the rest: it decodes with a
  symphonia that has no APE or WavPack decoder, in-process where a stuck
  decode cannot be killed; it hands over no integers, so no bit depth; it
  returns the cutoff and not the spectrum, which decision 8 draws; and it
  computes loudness, tempo, beats and key that nothing on the server reads.
- **Making `BandwidthMeter` public in the core** and calling it is the right
  end state, and "What is still open" records it as the change that retires
  the port. It is not the first step, because using it means bumping the core:
  the server pins `102df003`, the WaveFlow repository is 84 commits further,
  and that range changes the core's scanner (`scanner/extract.rs`,
  `scanner/folders.rs`). A quality page should not ship a scanner change.
- **An FFmpeg filter alone**: `aspectralstats` reports a roll-off, which is a
  percentile of energy and not a wall. A dull genuine recording
  has a low roll-off; the desktop's whole argument is that only a wall is
  evidence.

## Decision 3 — the verdicts, computed where they are read

The table stores measurements. The verdict is SQL, composed with `concat!`
like every projection here, in one macro, `quality_verdict_sql!`, so a
threshold change is a code change and never a re-analysis. It reuses the
core's `lossless_track_sql!` and `probable_fake_lossless_sql!` verbatim, with
the measurement table aliased `ta` as those macros expect, so the server and
the desktop cannot disagree about a fake lossless file.

In order, the first that applies:

| Verdict | When | Shown as |
| --- | --- | --- |
| `lossy` | a known lossy format: `MP3`, `Vorbis`, `Opus`, `Speex`, or `AAC` with no bit depth (an ALAC file has one) | "Lossy", the container's own grade |
| `not_measured` | any other format that `lossless_track_sql!` does not name: DSD, and any label the scanner may produce that neither list knows | "Not measured" |
| `pending` | lossless, with no measurement of the current content at the current version | "Waiting for analysis" |
| `unreadable` | FFmpeg failed or ran past its deadline on this content | "Could not be read" |
| `silent` | no frame above −60 dBFS | "Silent" |
| `lossy_source` | `probable_fake_lossless_sql!`: a wall at or under `MIN(21000, sample_rate / 2 − 1000)` | "Probably from a lossy file", with the cutoff |
| `upsampled` | `sample_rate > 48000` and a wall at or under 30 kHz | "Probably upsampled", with the cutoff |
| `padded` | `effective_bits < bit_depth` | "Padded to N bits", with the real depth |
| `consistent` | none of the above | "No sign of a lossy source" |

- **`lossy_source` before `upsampled`.** A 96 kHz file with a wall at 16 kHz
  came from an MP3 first; that it was upsampled afterwards is the lesser
  news. Above 21 kHz the two cannot be confused: no lossy encoder cuts
  there, and a resampler's filter does.
- **`upsampled` is new**, and its threshold is not the desktop's tuning. The
  first draft put it at 24 kHz, the Nyquist of a 48 kHz source, on the
  reasoning that a file above 48 kHz whose highs stop there holds nothing a
  48 kHz file could not. Measured, a resampler's filter does not stop dead
  at the source's Nyquist. On white noise through FFmpeg's own resampler,
  the wall stands at 21.5 to 23.8 kHz with a steep filter
  (`filter_size=64:cutoff=0.91`), and at 25.7 kHz from 44.1 kHz and 28.0 kHz
  from 48 kHz with its default one, whose transition runs past the source's
  Nyquist. The threshold is 30 kHz, which holds all of them. A genuine
  96 kHz recording of the same noise has no wall at all. The wall test is the
  same as for `lossy_source`, which is what protects a genuine hi-res
  recording that simply has little up high: it fades, it does not hit a
  wall. And the page says "probably".
- **What `upsampled` cannot see.** The same three minutes of real music
  through FFmpeg's default resampler show no wall at all: that filter lets
  through images of the music's own highs, mirrored above the source's
  Nyquist, and they come back within 15 dB of the level under the wall,
  which the desktop's rule reads, rightly, as content. Such a file is
  `consistent`. A steeper resampler leaves the wall, at 21.5 kHz on the
  same music, and is found. As with encoders that do not low-pass, no
  measure of where the highs stop finds what does not stop.
- **`padded` is the mildest**, and comes after the other two for that reason: a 16-bit file in a
  24-bit container is still a lossless CD. It is reported because it is a
  claim the container makes and the audio contradicts, and because it is how
  many fake hi-res files are built.
- **`consistent` is not "genuine".** The desktop's own measurements say a
  FLAC made from FFmpeg's AAC at 256 kbps or from Vorbis at q6 keeps the full
  band and has no wall. No measure of where the highs stop can tell those
  apart, so the honest label is "no sign of a lossy source", and the page
  says so once, in its help text.

The verdict comes with the numbers behind it: the cutoff in hertz, the fall
across the wall in dB, the file's Nyquist, the effective bits. A reader who
doubts the word can read the measurement, and decision 8 lets them see it.

## Decision 4 — in the background, bounded, never on the answering path

A `QualityAnalyzer` joins `AppState` beside the scanner. No request ever waits
for a measurement: every route reads what the table holds, and a file with
nothing there is `pending`.

**When it works.** It wakes at boot, when a scan job finishes, and when an
owner or manager asks for a library to be analysed again (decision 5). Each
time, it takes the available lossless tracks with no current measurement,
oldest first, in pages of 64, and works through them until none is left.
Nothing else wakes it, and it holds no timer of its own: a library that does
not change is not looked at again.

**How much at once.** `WAVEFLOW_QUALITY_PARALLELISM`, default **1**, files in
flight, each one FFmpeg process with `-threads 1`. So by default the analysis
costs **one core** at most, whatever the library's size. `0` turns the
analysis off: nothing is measured, every lossless file stays `pending`, and
the page says the operator disabled it. The setting is a field on `Config`,
as every tunable is.

**It yields.** While a scan job of any library runs, the analyser finishes the
files in flight and starts no new one until the scan is over. A scan already
saturates the cores it is given and takes the writer gate per batch; the
desktop learned to park its sweep during a scan for the same reason, after a
sweep racing a scan lost its results to `database is locked`. It does not
share the transcode semaphores: a listener's playback must never queue behind
a library's backlog, and the analysis is never urgent.

**How long one file may take.** A deadline of 60 seconds plus the track's own
duration, which asks the decoder for real time where it ran 650 times faster
here, so only a stuck process meets it, and never more than 30 minutes: the
duration is what the file's header claims, and a crafted file claiming ten
hours that hangs FFmpeg must not hold a slot for ten hours. After it, the process is killed
(`kill_on_drop`) and the file recorded `unreadable` for this content. A file
that cannot be read is not retried on every wake, which would loop forever on
one bad file; it is retried when its content changes or the detector's
version moves.

**Writes.** Each measurement is written as it finishes, in a transaction of
its own, under the writer gate, as every mutation is. The gate is never held
while FFmpeg runs. The first version wrote sixteen rows at a time, which on
a debug build left the page's count still for forty seconds and then
jumping. The count is what a reader watches, and a row a file costs one
short transaction against a decode a hundred times longer. A restart loses
at most the files in flight, and a row that cannot be written costs that
file and not the pass. Decision 5's guarantees are unchanged: a row is still
written only while the track holds the content it measured.

**Paths.** Every file goes through `secure_track_path` before FFmpeg sees it,
like a stream does. Audio files stay read-only: FFmpeg reads from the path
and writes only to its pipe.

**What it costs.** Measured with the prototype of decision 2 on this
instance (FFmpeg 8.0 on Windows, one thread, release build) over its 35 FLAC
files, 101 minutes of 44.1 kHz audio: 9.4 seconds in all, 0.27 s a file,
about 650 times faster than real time. At that rate a library of 10,000
lossless files of four minutes is about an hour of one core, once, and after
that only its new and changed files; a 96 kHz library costs about twice as
much per minute, because twice as many samples go through the same FFT. The
table in decision 6 holds about 300 bytes per file.

## Decision 5 — when a file is measured again

A measurement is **of a content, by a version, through a decoder**. It is
current when its `full_hash` equals the track's, its `version` equals the
analyser's, and its `decoder` equals the FFmpeg the server runs now.
Anything else is `pending`, and the next wake measures it.

- **A changed file at the same path** keeps its track id, since the scanner
  matches by path first, and gets a new `full_hash`: the old measurement stops
  being current at once, and the page shows `pending` rather than a verdict
  about bytes that are gone. This is how a fake replaced in place by its real
  rip loses its verdict.
- **A moved file** keeps its track id when the scanner relocates it, by its
  unchanged hash or, for a file both moved and re-encoded, by a relocation
  hint that names exactly one missing track. With an unchanged hash its
  measurement stays current; with a new one it is `pending`, as above. A rescan
  of an unchanged library re-measures nothing.
- **A file the scanner cannot relocate** becomes a new track with a new id.
  Nothing is carried over to it: it starts `pending` and is measured on its
  own. The old row's measurement stays with the old row, which is unavailable
  and lists nowhere, and goes with it when the row is deleted. No measurement
  is ever matched to a track other than the one whose content it measured.
- **A detector change** bumps `QUALITY_VERSION`, the server's own generation
  of what it stores: the ported wall search, the bit depth and the spectrum's
  bands. It starts at 1, and the port following a new `BANDWIDTH_VERSION` is
  one reason to bump it. Every lossless file becomes
  `pending` and is measured again, in the background, at the same bounded
  pace. A change to the verdict rules is not a detector change and bumps
  nothing, because the verdict is computed at read time (decision 3).
- **A different FFmpeg** re-measures too, without anyone bumping anything.
  The samples are FFmpeg's, and a decoder fix changes them for the files it
  fixes. The server already runs `ffmpeg -version` at start (`check_tool`,
  which today reads only the exit status). It keeps a SHA-256 of the **whole**
  output, not its first line: the first line names a release, while the rest
  names the build's configuration and the version of each library it runs
  with, `libavcodec` and `libswresample` included, each printed as compiled
  and as loaded. Two builds of one release with different decoders, or one
  binary over an upgraded shared `libavcodec`, therefore differ. Each
  measurement records the digest it was made under. An upgrade, a downgrade or a different build
  therefore makes every measurement `pending` again, and the background pass
  redoes them at its bounded pace, about an hour of one core for 10,000
  files (decision 4). Lossless decoders are bit-exact by design, so this
  mostly re-measures to the same numbers; it is the price of never showing a
  verdict about samples the current decoder would not produce.
- **A library analysed again on request** (`POST …/quality/analysis`, owner or
  manager): its measurements are deleted and its files queued. This is for an
  operator who doubts a result; it is never needed for correctness.

## Decision 6 — where the results live

One new dated migration:

```sql
-- One measurement per track: the audio, not the container (RFC-016).
CREATE TABLE track_quality (
    track_id TEXT PRIMARY KEY REFERENCES track(id) ON DELETE CASCADE,
    -- The content measured: the track's full_hash when it ran. A measurement
    -- whose hash is no longer the track's is about a file that has gone.
    full_hash TEXT NOT NULL,
    -- QUALITY_VERSION when it ran.
    version INTEGER NOT NULL CHECK (version > 0),
    -- SHA-256, in hex, of the whole `ffmpeg -version` output when it ran
    -- (decision 5): the release, the configuration and the library versions.
    decoder TEXT NOT NULL CHECK (length(decoder) = 64),
    outcome TEXT NOT NULL CHECK (outcome IN ('measured', 'silent', 'unreadable')),
    -- Where the wall stands, in hertz; NULL when the spectrum has none.
    cutoff_hz REAL,
    -- How far the level falls across the steepest candidate, wall or not.
    wall_db REAL,
    -- The bits the samples use; NULL for floating point, or all zero.
    effective_bits INTEGER CHECK (effective_bits BETWEEN 1 AND 32),
    -- The averaged spectrum (decision 8): 256 bands from 0 Hz to Nyquist,
    -- one byte each, in half-decibels under the loudest band.
    spectrum BLOB CHECK (spectrum IS NULL OR length(spectrum) = 256),
    -- Why a file was unreadable: 'decode' or 'timeout'. Never a message:
    -- FFmpeg's stderr can carry the path.
    failure TEXT CHECK (failure IN ('decode', 'timeout')),
    analyzed_at INTEGER NOT NULL,
    CHECK ((outcome = 'measured') = (spectrum IS NOT NULL)),
    CHECK ((outcome = 'unreadable') = (failure IS NOT NULL))
) STRICT;
```

- **Keyed by the track, not the hash.** The same bytes in two libraries are
  measured twice. Keying by hash would save that decode and make the
  measurement a fact shared across tenants, which then needs its own argument
  for why it leaks nothing; a duplicate decode costs a fraction of a second.
- **No `library_id`.** Tenancy reaches the row through `track`, which every
  read joins anyway (decision 7).
- **No verdict column**, for decision 3's reason.
- **The upgrade adds an empty table and schedules no rescan.** Every lossless
  file is `pending` after the upgrade, and the analyser's first wake, at boot,
  starts on them. No scan is needed, because the scanner already stored
  everything the analyser reads.

## Decision 7 — who may see it

**Any member of the library may read its quality.** A listener already
streams the file and already sees its codec, bitrate, rate and depth. Where
the highs stop is one more fact about the same file, and hiding it would
protect nothing. The routes join `library_member` in the query, as every
catalogue read does: a library or a track the caller cannot reach answers
`404`, identical to one that does not exist.

**Only an owner or manager may ask for an analysis again** (decision 5),
because it spends the operator's CPU, the rule RFC-002 applies to scans.
Anyone else gets the `404` of a foreign library.

**The verdict is not on the song item.** `GET /libraries/{id}/tracks`, the
album and artist pages and the Subsonic façade carry nothing new. The verdict
lives on the quality page, in the track's properties, and in "Needs
attention", following the desktop's reason: a false accusation printed on
every row of every list is worse than a true one found where someone went
looking. The frozen Subsonic contract is untouched; OpenSubsonic has no field
for this in any case.

**No library event.** A measurement is not a catalogue change, and the feed's
vocabulary (RFC-007) does not admit it. The page refreshes its own counts
while the analysis is running (decision 10).

## Decision 8 — the spectrum, and the spectrogram

Two pictures, for two questions.

**The averaged spectrum** is what the detector judged: level against
frequency, averaged over the whole file. It is stored (decision 6) as 256
bands from 0 Hz to Nyquist, each the mean power of its FFT bins in dB,
quantised to half-decibels under the loudest band, down to −127.5 dB. That is
256 bytes a file, enough to place a wall within about 86 Hz at 44.1 kHz and
375 Hz at 192 kHz, which is finer than any chart will draw it. The web client
draws it as a line, with the wall marked, and the 16, 19, 20 and 22 kHz
gridlines the reader wants to compare it against. It costs no request beyond
the track's own measurement.

**The spectrogram** is the picture people already use to judge a file by
eye (Spek, Audacity): frequency against time, colour for level. It shows
what an average cannot, such as a wall in one part of a file only. It is
**generated on demand**, when a reader asks for it, and never stored:

```text
ffmpeg -nostdin -loglevel error -threads 1 -i <guarded path>
       -filter_complex "[0:a:0]showspectrumpic=s=1024x512:legend=0:scale=log"
       -frames:v 1 -c:v libwebp -quality 70 -f webp pipe:1
```

That took half a second for a three-minute FLAC here and gives about 60 KB
of WebP; the same picture is 210 KB as JPEG and 870 KB as PNG. WebP needs
FFmpeg built with libwebp, which Debian's package (the one the Docker image
installs) and the common static builds are. The server asks FFmpeg's encoder
list once at start, beside the version check it already makes, and falls back
to JPEG, which every FFmpeg encodes, when libwebp is absent; the
`Content-Type` says which. It is answered `200` with `Cache-Control: private,
no-cache` and an `ETag` derived from the content hash and the picture's
parameters. `no-cache` lets the browser keep the picture and makes it ask
before every use: the server checks, as on any other request, that the
calling account may read the track, and only then answers `304` to a
matching `If-None-Match`. A picture kept for 24 hours without asking would
outlive a membership removed in the meantime, and could be shown to the next
account signed in to the same browser. The revalidation costs an access
check and no FFmpeg. It is bounded per account by one, and across the server
by `WAVEFLOW_QUALITY_PARALLELISM` plus one, so a reader can still have one
drawn while the background analysis runs; beyond that the route answers
`429` with `Retry-After`. Access is checked before the bound, so an account
that may not read the track gets `404` and never learns from a `429` that
something is there. It has a 20-second deadline, and less when the
request timeout leaves less, so the route answers its own `503` before the
router's timeout answers for it. Unlike the analysis,
it is on the answering path, because a picture someone clicked for is the
answer; that is why it is bounded and never computed for anyone who did not
ask. It is drawn for any file, lossy ones included, because a reader may want
to look at a lossy file too.

The web client fetches it with its bearer token, as it already fetches
artwork (`webapp/src/api/artwork.ts`), and shows the blob. It needs no ticket.

## Decision 9 — "Needs attention" gains its fake lossless files

RFC-015 decision 8 left `probable_fake_lossless` out because the server had
no analysis. It has one now, and the category comes back as the desktop
defines it: the tracks whose verdict is `lossy_source`. It is listed for
owners and managers, like every category on that page, and like the others
it offers no action of its own: it lists the tracks, and replacing the file
is the owner's business. `upsampled` and `padded` are not added: neither is
something an owner is asked to fix, and both are on the quality page for
anyone who wants them.

## Decision 10 — the page

A **Quality** page in the web client's library group, next to Folders and
"Needs attention", **shown to every member**, with the library scope the
Folders page already uses.

- **A header** gives the library's counts per verdict, as chips that filter
  the list, and the progress of the analysis: "1,204 of 3,310 lossless files
  analysed".
- **A status line** says what the analysis is doing, from the summary's
  `analysis`: running, with the count moving and the files in progress;
  waiting for the scan to finish; queued; or idle, with when the last file
  was analysed. It is a polite live region, so a screen reader hears the
  state change without being interrupted on every file. While the analysis
  works (running, queued or waiting), the page asks again every three
  seconds; idle or off, it stops asking. The header and the list are one
  answer: both are asked again on every visit, and a change of filter asks
  for the header beside the new list, so the counts above never contradict
  the rows below. The first version held both for the client's thirty
  seconds of cache, and a filter fetched a list fresher than its header.
- **The list** is the song table the other pages use, with three columns of
  its own: the container (`FLAC · 24 bit · 44.1 kHz · 1,021 kbps`, from
  `file-quality.tsx`), where the highs stop (`16.2 kHz`, or "full band"), and
  the verdict. Filters: verdict and format. Sorted by verdict, worst first
  (`lossy_source`, `upsampled`, `padded`, `unreadable`, `silent`, `pending`,
  `consistent`, then `lossy` and `not_measured`), then by cutoff, lowest
  first, `NULLS LAST`, so the most likely fakes open the page.
- **A row opens the track's properties**, which gain a "Real quality" section:
  the verdict in words, the numbers behind it, the averaged spectrum, and a
  "Show spectrogram" button.
- **For an owner or manager**, an "Analyse again" action on the library, with
  the second click RFC-015 already uses to confirm. It is offered only when
  the server, asked since the page was opened, says the analysis is idle:
  whether a pass runs is the server's to say, not something the page
  remembers, and a page left and opened again remembers nothing.
- **Each row shows the track's cover** before its title, as the song tables
  do: a title alone is hard to place.
- **The meter in `file-quality.tsx` does not change.** It grades the
  container, which is what it says it does. The track's properties put the
  measured verdict under it, so both are visible and neither pretends to be
  the other.

## The surface

All under `/api/v2`, tenancy-checked in the queries.

| Route | What |
| --- | --- |
| `GET /libraries/{id}/quality` | `{ enabled, lossless, measured, verdicts: [{ verdict, count }], analysis }`. Any member. `analysis` is `{ state, in_flight, pass_started_at, last_measured_at }`, below. |
| `GET /libraries/{id}/quality/tracks?verdict=&codec=&offset=&limit=` | The song items of `GET /libraries/{id}/tracks`, each with a `quality` object: `verdict`, `cutoff_hz`, `wall_db`, `nyquist_hz`, `effective_bits`, `analyzed_at`. Sorted as decision 10 says. `422` for an unknown verdict. Any member. |
| `POST /libraries/{id}/quality/analysis` | Decision 5. `202`. Owner or manager; `404` for anyone else. |
| `GET /tracks/{track_id}/quality` | One track's `quality` object, plus `spectrum`: the 256 bands as numbers in dB. `pending` with no numbers when nothing is current. Any member. |
| `GET /tracks/{track_id}/spectrogram` | Decision 8. `image/webp`, or `image/jpeg` without libwebp. Any member; `429` when the bound is reached. |

**`analysis.state`** is one of:

| State | When |
| --- | --- |
| `off` | The operator set `WAVEFLOW_QUALITY_PARALLELISM=0`. |
| `idle` | Nothing of this library is pending, and none of its files is being decoded. |
| `waiting_for_scan` | Files are pending, and a scan of a library the caller belongs to holds the analysis back. |
| `running` | Files are pending and a pass is running, including one held back by a scan the caller cannot see. |
| `queued` | Files are pending and no pass is running yet: the analyser is between passes. |

A scan holds every measurement back, of every library, but only a scan the
caller can see is named. A scan of somebody else's library is not something
to confirm to them, so it changes nothing they are shown: the pass still
reads as `running`, and the file it picked as in flight, exactly as with no
scan at all. A file counts as in flight from the moment the pass picks it,
before any wait, for the same reason. `in_flight` counts this library's
files picked and not yet written, `pass_started_at` is set only while
`running`, and `last_measured_at` is the latest `analyzed_at` among this
library's current measurements.

Every read goes through `DomainServices`, so the desktop can reach the same
answers when it chooses to, and so can a future Subsonic extension if one is
ever specified.

## What this RFC changes elsewhere

- **`src/scanner.rs`**: one line, a wake of the analyser when a scan job
  finishes. The scanner reads nothing new.
- **`src/media.rs`**: `secure_track_path` becomes `pub(crate)`, so the
  analyser and the spectrogram read files through the guard streams use, and
  the start-up check of `ffmpeg -version` keeps a digest of its whole output
  (decision 5). The transcode semaphores are not shared.
- **`src/services/attention.rs`**: the `probable_fake_lossless` category.
- **`src/config.rs`**: `WAVEFLOW_QUALITY_PARALLELISM`.
- **The Subsonic façade**: nothing. `docs/subsonic-compatibility.md` needs no
  row.
- **The desktop app** (`InstaZDLL/WaveFlow`): nothing. Its own analysis keeps
  running on its own copies. Reading a server's verdicts is its own change,
  against the routes above.
- **CLAUDE.md**: `track_quality` and the analyser are mentioned beside the
  scanner in "Architecture & conventions", and the new integration target is
  added to the list of targets.
- **Migrations**: one new dated file creating `track_quality`, empty. None
  edits an applied migration.

## The order of the work

1. **This RFC.**
2. **The measurement**: the migration, `src/quality/` (the ported detector,
   the effective bit depth, the FFmpeg pipe, the 256-band spectrum), the
   `QualityAnalyzer` and its wakes, `WAVEFLOW_QUALITY_PARALLELISM`, and the
   verdict macro. Tests, in a new `quality` integration target:
   - the constants and parity tests of decision 2;
   - a FLAC made from an MP3 at 128 kbps measures `lossy_source` with a
     cutoff near 16 kHz, and the FLAC it was made from measures `consistent`;
   - a 44.1 kHz file resampled to 96 kHz measures `upsampled`, and shaped
     noise at 96 kHz with a full band does not;
   - a 16-bit file written as 24-bit measures `padded` with 16 effective bits;
   - a changed file goes back to `pending` and is measured again; a moved one
     keeps its measurement; a measurement recorded under another `decoder`
     is `pending`; two `ffmpeg -version` outputs with the same first line and
     a different configuration or `libavcodec` line give different digests;
   - a file FFmpeg cannot decode is `unreadable` once and not retried until it
     changes;
   - a scan in progress holds the analyser back until it finishes;
   - each test that changes a rule is checked by inversion, the rule removed
     and the test seen to fail.
3. **The routes**: the five routes, the `probable_fake_lossless` category, and
   their tests: a listener reads the quality and the spectrogram and cannot
   ask for an analysis; a non-member gets `404` on every route; the
   spectrogram answers an image of the type it names, and `429` past its
   bound; a request carrying the `ETag` gets `304` from a member and `404`
   from an account that is not one, or no longer is.
4. **The page**: the Quality page, the track properties' "Real quality"
   section with the averaged spectrum and the spectrogram, and the category
   on "Needs attention".

Steps 2 to 4 can land as one pull request; each step is also shippable alone,
since step 2 fills a table nothing reads, and step 3 serves it.

## What is still open

- **Lossy files re-encoded at a higher bitrate.** A 320 kbps MP3 whose highs
  stop at 16 kHz was a 128 kbps MP3 first. Finding it means measuring lossy
  files too, about four in five files on this instance, and a table of the
  cutoff each encoder uses at each bitrate, which varies between LAME, FhG,
  Apple's AAC and Opus. The table here has room for it; the verdict needs its
  own measurement first.
- **The core's meter.** When the server next bumps `waveflow-core`, a change in
  the WaveFlow repository making `BandwidthMeter` public would let the server
  call it directly and delete the port. The parity test of decision 2 is what
  tells that change it is safe.
- **Encoders that do not low-pass.** A FLAC made from FFmpeg's AAC at 256 kbps
  or Vorbis at q6 keeps its highs, and nothing in this document finds it.
  Finding those means looking for the encoder's quantisation in time and
  frequency rather than for a wall, which neither the desktop nor this
  document attempts.
- **A quality rule for smart playlists.** The desktop has `fake_lossless` as a
  rule. The server has no smart playlists yet.
