# RFC-026 — No jukebox mode

- **Status**: Proposed
- **Implemented by**: [#416](https://github.com/InstaZDLL/waveflow-server/pull/416)
  (this RFC; the decision is to build nothing, so no code follows it). The
  *Status* field above never flips in this project; this line is the one that
  says what is running.
- **Date**: 2026-11-02
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md) (the frozen Subsonic
  contract)

## Problem

In jukebox mode the server plays audio on its own output device, and a Subsonic
client drives it through `jukeboxControl`. It is the last of Navidrome's server
features WaveFlow does not have ([gap analysis](../analyses/web-client-gap-analysis.md)),
and issue [#402](https://github.com/InstaZDLL/waveflow-server/issues/402) asked
for a decision before any code.

Today the façade declares `jukeboxRole="false"` (`src/subsonic/nodes.rs`), and
`jukeboxControl` falls through the dispatch to the generic "Requested method is
not implemented" error (`src/subsonic/mod.rs`).

## Decision — WaveFlow does not build it

The server stays something that serves audio to the device a person is holding.
`jukeboxRole` stays `false`, `jukeboxControl` stays unrouted, and no configuration,
table or dependency is added. The Subsonic contract does not change, because
those two answers are the ones clients already get.

## Why

**The installs it would serve barely exist.** Most servers run in a container on
a machine with no speakers, where the feature could not produce a sound. The
standalone download runs on a machine that has them, but there the web client
already plays on that same machine.

**Neither way of reaching the device comes free.**

- FFmpeg is built with no output device at all. `packaging/ffmpeg/build.sh`
  compiles with `--disable-everything` and fails the build if an output device
  appears (`check_components OUTDEV ""`). Adding one per platform would widen a
  build kept minimal on purpose
  ([standalone download plan](../plans/standalone-download-plan.md), §3).
- A playback crate in the binary has a cost on Linux. `cpal`, the usual choice,
  reaches the device through ALSA's `libasound`, whereas the Linux server of the
  standalone archives is static against musl so that it runs on any distribution
  (`.github/workflows/standalone.yml`). This has not been measured: it is why the
  option looks expensive, not a proof that it cannot work.

**It would add a kind of state the server does not have.** One queue and one
output shared by every account needs a role, an answer for two people sending
commands at once, and a process that keeps playing with no client attached.
Playback today belongs to one account and one request.

## If this is reopened

A new RFC, not an edit to this one. It would have to answer what #402 listed: how
audio reaches the device, who may drive it, and what happens when two accounts
send commands. It would also have to keep the feature off by default, with
`jukeboxRole` remaining `false` until an operator turns it on, so that the
observable contract does not move on an existing install.
