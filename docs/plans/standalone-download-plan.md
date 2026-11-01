# A standalone download: the server on a laptop, without Docker

> Plan of 2026-10-29, written against `main` at `4c32b51`, while 2.0.0 is
> prepared as a container image for `linux/amd64` and `linux/arm64`.
>
> **Status as of 2026-11-01:** steps 1 to 5 of §8 are built. FFmpeg is
> found beside the executable (§4), and laptop mode exists in the server:
> `WAVEFLOW_LAPTOP`, read from the `laptop.env` beside the executable, moves
> the data folder and the cache size, checks the port before the boot,
> prints the banner, and opens the browser — on a first start on Windows and
> macOS on the sign-in screen with the setup token in the URL fragment (§5,
> #397); the file the archives carry is `packaging/standalone/laptop.env`. The
> source link §13 asks for is built too (#384, §3). The minimal FFmpeg of §3 is
> built by
> `packaging/ffmpeg/build.sh` and `.github/workflows/ffmpeg.yml` for the five
> targets, each tested against the server (#398), and the archives ship it
> (#399, §7); the image keeps Debian's FFmpeg, now trixie's 7.1 (#395). No tag
> has produced the archives yet: the check that each one, extracted on its
> platform, starts in laptop mode and plays a file waits for the first release
> candidate. Not built yet: signing (step 6, #401).

The question this plan answers: can someone who has never opened a terminal
download one archive, double-click what is inside, and end up in the web
player with their music, on Windows, macOS or Linux?

Almost. The binary is already self-contained — SQLite is compiled in, the web
client is embedded by `rust_embed`, there is no service to install beside it.
Three things stand in the way, and this plan takes them in turn: FFmpeg has to
be on `PATH` (§2 to §4), every setting is an environment variable with
server-room defaults (§5), and an unsigned executable downloaded from the
internet is something both Windows and macOS refuse to open without a fight
(§6).

## 1. What stays as it is

- **One binary, the same one.** No second build, no feature flag: the
  standalone archive carries the binary the image carries, compiled for the
  host. Laptop mode (§5) is a set of defaults chosen at boot, not a different
  program.
- **The image is still the server-room install.** Nothing here changes how
  Docker, systemd or a reverse proxy run WaveFlow. Every default in §5 applies
  only when laptop mode is asked for.
- **No launcher process.** Tauri and Bun ship a sidecar beside the program that
  runs it; here the server *is* the program, so the sidecars are the tools it
  runs, and nothing sits between the user and the server.

## 2. What each archive contains

| Archive | Contents |
|---|---|
| `waveflow-server-<version>-windows-x86_64.zip` | `waveflow-server.exe`, `ffmpeg.exe`, `ffprobe.exe`, `laptop.env`, `LICENSES/`, `README.txt` |
| `waveflow-server-<version>-macos-arm64.zip` | `waveflow-server`, `ffmpeg`, `ffprobe`, `laptop.env`, `LICENSES/`, `README.txt` |
| `waveflow-server-<version>-macos-x86_64.zip` | the same, for Intel Macs |
| `waveflow-server-<version>-linux-x86_64.tar.gz` | `waveflow-server`, `ffmpeg`, `ffprobe`, `laptop.env`, `LICENSES/`, `README.txt` |
| `waveflow-server-<version>-linux-arm64.tar.gz` | the same, for a Raspberry Pi or an ARM laptop |

- **Flat, one folder.** The three executables sit side by side, because that is
  where the server looks (§4). The archive extracts into a folder named after
  itself, so "extract here" does not scatter files across Downloads.
- **`laptop.env`** turns laptop mode on and lists, commented out, the settings a
  laptop user may want to change: the port, LAN access, the data folder (§5).
- **`LICENSES/`** holds `COPYING` (AGPL-3.0 for the server), FFmpeg's licence
  and the licences of the libraries compiled into it, and `SOURCE.txt`, which
  says where the corresponding source of each is (§3).
- **`README.txt`** is the page a non-technical user reads: what to double-click,
  what the first screen asks for, where the music library is set, how to stop
  the server, and what to click past on first launch while the archives are
  unsigned (§6).
- **Linux is `tar.gz`, not `zip`:** a zip loses the executable bit on most
  extractors. An AppImage is the Linux equivalent of a double-clickable file
  and is noted in §8 as a later option, not part of the first archives.
- **No Windows on ARM, no 32-bit.** A Windows ARM laptop runs the x86_64 build
  under emulation; a native one is added when someone asks for it.

## 3. Where FFmpeg comes from, and under which licence

### The licence question

The server is AGPL-3.0-only. FFmpeg is LGPL-2.1-or-later by default, GPL when
configured with `--enable-gpl` (to link x264, x265 and other GPL-only
libraries), the version-3 variants with `--enable-version3`, and not
redistributable at all with `--enable-nonfree` (fdk-aac among others).

The server never links FFmpeg: it runs `ffmpeg` and `ffprobe` as separate
processes and talks to them through arguments and pipes. Shipping them in the
same archive is aggregation, so the AGPL and FFmpeg's licence do not have to
be compatible with each other — either GPL or LGPL builds could legally ship
beside the server. What the choice changes is what **we** owe for distributing
FFmpeg's binaries:

- the licence text of FFmpeg and of every library compiled into it;
- the **corresponding source** of every copyleft part, for the exact build
  shipped. GPLv2 §3 and GPLv3 §6(d) both accept "equivalent access to copy the
  source code from the same place" as the binary, which on GitHub means the
  source tarballs attached to the same release as the archives. That replaces
  a written offer valid three years, which is the alternative and a promise we
  would have to keep.

### The decision: our own LGPL build, minimal

WaveFlow needs no GPL code from FFmpeg. What it actually asks of FFmpeg, read
from the code:

- **decode** every format the scanner indexes — MP3, FLAC, ALAC and AAC in MP4
  and raw AAC, Vorbis, Opus and FLAC in Ogg, PCM in WAV and AIFF, DSD (`dsf`,
  `dff`) — for transcoding and for the quality analysis (`src/quality/`). The
  list is `waveflow-core`'s `AUDIO_EXTENSIONS`; WavPack and APE, which an
  earlier draft of this page named, are not in it. Speex in Ogg is the one
  stream an indexed extension can carry that this build does not decode; the
  desktop's engine refuses it as well;
- **encode** MP3 through `libmp3lame` (LGPL), Opus through `libopus` (BSD), AAC
  through FFmpeg's own encoder (`src/media/transcode.rs`), the analysis's raw
  `pcm_s32le` and its spectrum image through `mjpeg`, or through `libwebp`
  (BSD) when it is there (`src/quality/`, `probe_webp` in `src/app/boot.rs`);
- **write** the containers those outputs go into: the `mp3`, `ogg` and `adts`
  muxers of the transcode, `s32le`, `image2pipe` and `webp` of the analysis,
  and `mp4` and `webm`, into which `strip_canvas_audio` copies a canvas's video
  stream without re-encoding it (`src/services/canvas/probe.rs`);
- **probe** canvases, which are short H.264, HEVC or VP9 loops in MP4 or WebM:
  the server reads only their container, their duration and which streams
  they carry, then copies the video stream, so no video decoder is needed.
  Embedded covers are read by the tag library, never by FFmpeg;
- the filters the transcode and the analysis use (`-af` for ReplayGain,
  `-filter_complex` for the spectrum walk);
- the `file` and `pipe` protocols, and **nothing else**: RFC-023 already
  restricts every invocation with `-protocol_whitelist file`, and a build with
  no network protocol compiled in makes that true by construction.

Every one of those is LGPL or more permissive. So the archives carry an
**LGPL build of FFmpeg, configured with `--disable-everything` and only the
components above enabled**, built in CI from the pinned release tarball of
FFmpeg and of LAME, Opus and libwebp, with one script for all five targets:

- **Size.** A general-purpose static build is around 100 MB per tool; a
  build of what WaveFlow uses is a small fraction of it. The download is the
  first thing a non-technical user waits for.
- **The source offer is four tarballs and a script.** A general-purpose static
  build links dozens of libraries; each copyleft one among them is source we
  would have to mirror for every release, for the exact version linked.
- **The same FFmpeg on every platform**, so a transcode or a quality verdict
  does not differ between a Mac and a Windows laptop. `track_quality.decoder`
  records the FFmpeg digest, so a change of FFmpeg re-measures, as it should.
- **No trusted LGPL build exists for macOS.** The common macOS downloads are
  GPL builds; we would build there anyway.

The list of enabled components is not trusted from this page: the job that
builds FFmpeg runs the `media`, `quality` and `canvas` integration targets
against it before anything is published, so a format the build forgot fails
the release rather than a user's library.

**Considered and set aside: [BtbN's FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds).**
Its `lgpl` variant for `win64`, `linux64` and `linuxarm64` is LGPLv3 and does
include `libmp3lame`, `libopus` and `libwebp` (checked against its
`scripts.d` on 2026-10-29). It is the fallback if our own build is late: pinned
to one `autobuild-…` release by checksum, with its build scripts and the
source of each LGPL library it links attached to our release. It does not
cover macOS, and it carries the size and source-offer cost above.

### What ships with it

- `LICENSES/FFmpeg.txt` — FFmpeg's LGPL text, and the GPL text the LGPLv3
  incorporates when the version-3 variant is used;
- `LICENSES/LAME.txt`, `LICENSES/Opus.txt`, `LICENSES/libwebp.txt`;
- `LICENSES/musl.txt` in the Linux archives, whose static binaries carry musl;
- `LICENSES/MinGW-w64-runtime.txt` and `LICENSES/winpthreads.txt` in the Windows
  archive, whose FFmpeg links both statically (the paths of winpthreads' sources
  can be read in the binary, though its own threads are `w32threads`). The first
  is the file the runtime's licence note says to copy into a package. They are
  copied from the MSYS2 packages the build linked. GCC's own runtime is under its
  Runtime Library Exception, which asks for nothing;
- `LICENSES/SOURCE.txt` — the FFmpeg configure line, the version of each
  component, and the names of the source tarballs attached to the release;
- on the release itself, `ffmpeg-<version>-sources.tar.gz`: the four upstream
  tarballs exactly as downloaded (with their checksums), the build script and
  the musl notice it copies.

### The server's own source

The archive is also object code of the server, so AGPL §6 asks for its
corresponding source, for every version shipped — not only the server's
repository at the tag, but the exact source of everything compiled into the
binary: every non-system dependency `Cargo.lock` pins (415 crates from
crates.io today), and `waveflow-core` at the revision `Cargo.toml` pins
(`rev = "…"` on `InstaZDLL/WaveFlow`, the one git dependency in the lock
file).

§6(d) allows that source to sit on another server, provided clear directions
next to the object code say where. So no separate archive is attached for it:
`LICENSES/SOURCE.txt` names the tag, whose source archive carries
`Cargo.lock` and `webapp/bun.lock`; the `waveflow-core` revision and its public
repository; crates.io for the crates, where a published version stays
downloadable even once yanked; and npm for the packages of the web client, which
is compiled into the binary too. Those are durable public links, and they stay
our responsibility: a dependency taken from anywhere less durable — a second git
dependency, a private registry — has its source attached to the release instead.
`packaging/standalone/pack.sh` refuses a `Cargo.lock` that names any other
source, so that happens at the release rather than by someone remembering.

§13 asks more of a network service: the server must offer its source to the
people who use it remotely. **It does, since #384.** `/health` reports `source`, the repository at the tag `v<version>` of
the running binary, never `main`; the web client links to it under the sign-in
form, which every remote user reaches, and under every section of Settings;
and the OpenAPI document carries the licence's URL at that tag and the same
address as its `externalDocs`. The address is built once, in
`src/app/source.rs`, from the manifest's `repository` and `version`, so a
modified build that names its own repository in `Cargo.toml` offers its own
source everywhere at once. The tag is the entry to the complete offer above:
its archive carries `Cargo.lock` and the `waveflow-core` revision.

One limit remains: a build from an untagged commit between two releases
reports the version of the last release and links to that tag, which is not
the source it runs. Releases are what get distributed, so the offer holds for
everything shipped; naming the commit as well would need the build to record
it.

## 4. How the server finds FFmpeg — built with this plan

`WAVEFLOW_FFMPEG_PATH` and `WAVEFLOW_FFPROBE_PATH` used to default to the bare
names, for the system to find on `PATH`. The order is now
(`src/config/tools.rs`):

1. the setting, when the operator set one — the Docker image names
   `/usr/bin/ffmpeg` this way, and a self-built FFmpeg stays theirs;
2. a regular file named `ffmpeg` (`ffmpeg.exe` on Windows) **in the directory
   of the running executable**, resolved through symbolic links on Unix so a
   binary linked into `/usr/local/bin` still finds its sidecar;
3. the bare name, on `PATH`, as before.

A sidecar that is present but broken is still chosen, and the boot refuses it
by its full path. Falling back to `PATH` would have hidden that the archive's
own copy never ran, and on a laptop with a different FFmpeg installed the
transcodes would quietly come from a build the release never tested.

Nothing else changes for an existing install: a server whose directory holds
no `ffmpeg` resolves exactly as before.

## 5. Laptop mode

### How it is switched on

`laptop.env`, beside the executable, is read at startup — through `dotenvy`,
as `.env` in the working directory already is, and with variables already in
the environment winning. It sets `WAVEFLOW_LAPTOP=true` and documents, commented
out, the few settings a laptop user may change.

Not the presence of FFmpeg beside the binary, and not "no variable is set": a
systemd unit that relies on today's defaults (`./data`, `127.0.0.1:4533`) sets
nothing either, and must not wake up one day with its data directory moved.
Laptop mode is asked for, by a file the archive ships and an operator never
has.

`WAVEFLOW_LAPTOP` is an ordinary setting, read in `src/config/` like every
other. It changes **defaults** only; any variable set explicitly still wins.

### What it changes

| Setting | Default | In laptop mode |
|---|---|---|
| `WAVEFLOW_DATA_DIR` | `./data`, relative to wherever the process starts | the OS data folder: `%LOCALAPPDATA%\WaveFlow Server` on Windows, `~/Library/Application Support/WaveFlow Server` on macOS, `$XDG_DATA_HOME/waveflow-server` (or `~/.local/share/waveflow-server`) on Linux |
| `WAVEFLOW_BIND` | `127.0.0.1:4533` | the same: this computer only |
| `WAVEFLOW_TRANSCODE_CACHE_MAX_BYTES` | 10 GiB | 2 GiB — a laptop disk is not a NAS |
| `WAVEFLOW_LOG_FORMAT` | text | text, with a one-line banner (below) |
| opening a browser | never | on every start (below) |

- **The data folder is the local one.** `%LOCALAPPDATA%` rather than
  `%APPDATA%`: a roaming profile would copy the database and the transcode
  cache to every machine the user signs in to. A double-clicked binary in
  Downloads must not write `data/` into Downloads either, which is what the
  relative default would do.
- **Localhost by default; the LAN on purpose.** Listening on every interface
  is what lets a phone on the same Wi-Fi reach the server, and is the reason
  the setup token exists (RFC-019). `laptop.env` carries
  `# WAVEFLOW_BIND=0.0.0.0:4533` with a sentence saying what uncommenting it
  does; Windows' firewall will then ask once. Laptop mode leaves
  `WAVEFLOW_PUBLIC_URL` unset, and without it the browser origin check falls
  back to the request's `Host` header (the boot says so in a warning), so a
  phone browsing to `http://<laptop's address>:4533` signs in. Setting a public
  URL would break exactly that, since a laptop on Wi-Fi has no stable address
  to name. Subsonic clients send no `Origin` and are not concerned either way.
  The boot's warning about the missing public URL is aimed at an operator and
  is noise on a laptop; laptop mode should say instead which address the LAN
  can use.
- **A fixed port, never a random one.** A Subsonic client on the phone, and the
  user's bookmark, keep the address. If 4533 is taken, the server asks
  `/health` on it: a WaveFlow answering means this one is already
  running, so the browser is opened on it and the new process exits. Anything
  else is a refusal that names the port and the line of `laptop.env` to
  change. Navidrome also defaults to 4533, so this will happen.
- **The browser opens on every start**, at `http://127.0.0.1:4533/`. On the
  first start, while setup is open, at
  `http://127.0.0.1:4533/login#setup-token=<token>`: a fragment is never sent
  to the server, so it is in no request line and no trace. The sign-in screen
  itself rather than the root, because the root's client-side redirect to
  `/login` drops the fragment. The sign-in screen moves it into the token
  field and removes it from the address bar with `history.replaceState`. The
  token is still drawn and written to `<data>/setup-token` exactly as today;
  the fragment is a hand-over and the file stays the source. A second copy
  opens the bare address, never the token: it knows what holds the port only
  by a `/health` body any local program can imitate, and the file outlives a
  first copy that stopped before setup. The first copy hands it over only on
  Windows and macOS, which keep a process's command line from other accounts
  (Linux and the BSDs show it to anyone, while the file is its owner's alone),
  and only when it listens on the address it opens: behind a wildcard such as
  `0.0.0.0`, another program may bind the loopback address more specifically.
  Elsewhere it opens the bare address. `WAVEFLOW_LAPTOP_OPEN_BROWSER=false`
  turns the opening off. *Built (#397)*, through each platform's own opener —
  `rundll32`'s URL handler on Windows rather than `cmd /c start`, whose
  command line would split on `&` and expand `%` — so no crate was added.
- **The console window is the server.** On Windows and macOS (where a
  double-clicked executable opens in Terminal) the window prints one banner —
  the address, the data folder, "close this window to stop the server" — and
  then the ordinary log. Closing it stops the server. A tray icon is a later
  option (§8), not a requirement of the first archives.
- **The library** is created from the administration page, as today; the
  folder picker browses the laptop's own disk, which is what the user expects.
  Suggesting the OS Music folder on the first visit is a small follow-up in the
  web client, not a server default.

### What it does not change

The schema, the APIs, the Subsonic façade, the instance key and the backups
are the same: a laptop install can be moved to a server later by copying the
data folder, as documented in the operations guide.

## 6. Signing

Until the archives are signed, the first launch shows a warning on Windows and
macOS, and `README.txt` says what to click. That is acceptable for a beta; not
for a download offered to people who have never seen the warning.

### Windows: SmartScreen

- An unsigned `.exe` from the internet gets "Windows protected your PC", with
  the run button behind "More info". Signing with an Authenticode certificate
  removes "unknown publisher"; SmartScreen's warning itself disappears only as
  the certificate builds reputation — an EV certificate no longer skips that.
- **Azure Artifact Signing** (formerly Trusted Signing) is the cheapest
  managed option, but Microsoft's documentation limits it to organisations in
  the US, Canada, the EU and the UK and to *individual* developers in the US
  and Canada; an individual maintainer in the EU does not qualify today.
- **Decision: apply to the [SignPath Foundation](https://signpath.org/)**, which
  signs open-source releases for free from a CI pipeline, with the key in its
  HSM. Its conditions — an OSI licence, a public build from the tagged source,
  no proprietary components — the project already meets. A commercial OV
  certificate is the fallback.
- All three executables are signed: `ffmpeg.exe` and `ffprobe.exe` are
  executables the user runs indirectly, and an unsigned one would be flagged
  by the same reputation system on its own.

### macOS: Gatekeeper and notarization

- A quarantined unsigned binary is refused. Since macOS 15 the Control-click
  bypass is gone: the user must open System Settings › Privacy & Security and
  choose "Open Anyway". That is the page `README.txt` explains while the
  archives are unsigned.
- Signing needs an **Apple Developer Program** membership (99 USD a year) and a
  *Developer ID Application* certificate. Every Mach-O in the archive is
  signed with the hardened runtime (`codesign --options runtime --timestamp`),
  then the zip is submitted to `notarytool` and the job waits for the verdict.
- **A bare executable cannot be stapled**: stapling works on an `.app`, a
  `.dmg` or a `.pkg`. A notarised zip of bare binaries still passes Gatekeeper,
  which checks the ticket online at first launch; offline, it does not. So the
  first macOS archive is a notarised zip, and the step after it is a `.app`
  bundle in a stapled `.dmg` — which also gives macOS users a real
  double-click instead of a Terminal window (§8).
- The certificate and the App Store Connect API key live in repository secrets,
  used only by the release job, as the Docker Hub token is.

### Linux

No signing gate exists for a downloaded binary. The checksums file is signed
instead (below), which also covers the other platforms for anyone who checks.

## 7. What `release.yml` adds

*Built (#399).* `release.yml` calls `.github/workflows/standalone.yml` after
`verify`, beside the image's build; that file calls `ffmpeg.yml`. Nothing in
either publishes, so a pull request that touches `packaging/standalone/` or a
manual dispatch runs the same jobs and stops at a workflow artifact, which is
how they are exercised without a tag. In order:

1. **`ffmpeg`** — `ffmpeg.yml` as it stands: one native runner per target
   builds the minimal FFmpeg, runs the `media`, `quality` and `canvas` targets
   with it first on `PATH`, and uploads the two tools and their `LICENSES/`.
   One job fetches the four tarballs, packs the source bundle from them and
   hands them to the others as an artifact, so a target never meets a mirror
   and the binaries are built from the bytes the bundle carries. Should
   upstream refuse that one job, it asks Alpine's copies of the same tarballs,
   and the script's SHA-256 holds either to the pin. Nothing is cached, despite
   what this page first proposed: what a release ships is compiled from its
   tag, and its build costs minutes, on a day that is rare.
2. **`webapp`** — the client, once, for all five.
3. **`archive`** — one native runner per target builds the server, packs the
   archive of §2 with `packaging/standalone/pack.sh`, and starts it with
   `packaging/standalone/smoke.sh`: the files of §2, nothing loaded that the
   system lacks, laptop mode from the `laptop.env` in the archive, and an empty
   `PATH`, so the tools can only be the ones beside it, then a FLAC file turned
   into an MP3 through the Subsonic façade. macOS builds on a runner of each
   architecture rather than cross-compiling, as `ffmpeg.yml` does.

   Two builds differ from the image's, because a laptop is not a container
   whose libraries we choose. **On Linux the server is static against musl**,
   built inside Alpine like its FFmpeg, and the same three targets run again on
   it: a binary built on `ubuntu-24.04` would refuse Ubuntu 22.04, Debian 12 and
   every other older glibc. **On Windows the C runtime is linked statically**
   (`+crt-static`): `VCRUNTIME140.dll` is not part of Windows, and a laptop that
   has never installed the redistributable would refuse the executable before
   it printed a line.
4. **`assets`** — one `SHA256SUMS` over the five archives and the source bundle,
   and all seven files in one artifact.
5. **The release job** attaches that artifact, and the release notes gain a
   "Download" section above the image's. It runs last, and the image's tags now
   wait for the archives too, so a tag whose archives failed announces nothing.

A pre-release tag publishes the archives as well, marked by the release's own
pre-release badge as the image is.

Not built: the keyless Sigstore signature of `SHA256SUMS` (`cosign sign-blob`)
this page proposed, which §6 still counts on for Linux. Neither #399 nor #401
lists it.

## 8. The order of the work

Each line is one pull request.

1. **Find FFmpeg beside the executable**, and this plan. *Done here.*
2. **`WAVEFLOW_LAPTOP` and `laptop.env`**: the defaults of §5, the OS data
   folder, the single-instance check, the banner. Server only, testable
   without packaging. *Done.* A second copy that finds WaveFlow on the port
   prints its address and exits; opening the browser on it is step 3.
3. **The setup-token fragment** in the sign-in screen, and opening the
   browser on start. *Done (#397).* A second copy opens the browser on the
   first, without the token, and so does a first copy on Linux or behind a
   wildcard address.
4. **The minimal FFmpeg build** script and its CI job, tested against the
   three integration targets on all five platforms. *Done* (#398):
   `packaging/ffmpeg/build.sh` pins FFmpeg 8.1.3, LAME 3.100, Opus 1.6.1 and
   libwebp 1.6.0 by SHA-256, fails if configure leaves out any component it
   lists or enables one it does not (those a listed component or the `ffmpeg`
   tool selects are named), or lets in any protocol but `file` and `pipe`, and
   writes `LICENSES/`, `SOURCE.txt` included, beside the two tools; `sources`
   packs the four tarballs and the script. `.github/workflows/ffmpeg.yml`
   builds on a native runner per target — Linux inside Alpine, so the binaries
   are static against musl — and runs the `media`, `quality` and `canvas`
   targets with the build first on `PATH`. Each tool weighs 5.5 to 8 MB. The release job of §7 calls this
   workflow, which also stays as the check of a change to the script.
5. **The standalone archives and `SHA256SUMS`** in `release.yml`, unsigned,
   with `README.txt` saying what to click past; the header rewritten. *Done*
   (#399), §7 says how; the first release candidate is what checks each archive
   on a real desktop.
6. **Signing**: SignPath for Windows once accepted, Developer ID and
   notarization for macOS once the membership exists.
7. Later, as asked for: a macOS `.app` in a stapled `.dmg`, an AppImage, a tray
   icon on Windows and macOS, and an update notice in the web client.
