# Documentation

Everything about WaveFlow Server v2 that is not code, filed by what each
document is. Start with the guide for the surface you are working against; the
RFCs say why it is that way.

## Guides

What a client author or an operator reads first.

| Document | For |
|---|---|
| [Native API v2 guide](guides/api-v2-guide.md) | Clients of `/api/v2`: authentication, sessions, the resources and their errors. The full route list is the server's own `/reference` page. |
| [Subsonic and OpenSubsonic guide](guides/subsonic-api-guide.md) | Clients of the `/rest` façade: authentication, the methods served, the extensions, and where WaveFlow differs from the reference server. |
| [Running a WaveFlow Server](guides/operations.md) | Operators: the first account, backups and the instance key, reverse proxies, the optional features, scanning, older Subsonic clients and the probes. |
| [What leaves the server](guides/privacy.md) | Operators: every outbound request the server can make, and what it sends. |

## Compatibility

| Document | What it records |
|---|---|
| [Subsonic compatibility matrix](compatibility/subsonic-compatibility.md) | The real clients run against the façade, each run read back from the server, and every deviation and correction those runs led to. |
| [M3 — Symfonium validation](compatibility/M3-symfonium-validation.md) | The milestone record of the first Symfonium validation and its replayable procedure. Dated, and in French on purpose. |

## RFCs

The design decisions, one per document. A `Status` line never changes; the
`Implemented by` line names the pull requests that did the work.

| RFC | Subject |
|---|---|
| [RFC-002](rfcs/RFC-002-waveflow-server-v2.md) | WaveFlow Server v2: the architecture, and the frozen Subsonic contract |
| [RFC-003](rfcs/RFC-003-waveflow-sync-v2.md) | The desktop's user-data sync |
| [RFC-004](rfcs/RFC-004-local-server-reconciliation.md) | Reconciling a local library with a server |
| [RFC-007](rfcs/RFC-007-library-event-stream.md) | The library event stream |
| [RFC-008](rfcs/RFC-008-receiving-a-file.md) | Receiving a file |
| [RFC-009](rfcs/RFC-009-track-canvas.md) | A track's canvas |
| [RFC-010](rfcs/RFC-010-external-scrobbling.md) | External scrobbling |
| [RFC-011](rfcs/RFC-011-metadata-enrichment.md) | Server-side metadata enrichment |
| [RFC-012](rfcs/RFC-012-library-browser.md) | The library browser: folders as a view, filing as metadata |
| [RFC-013](rfcs/RFC-013-chosen-artist-imagery.md) | Chosen artist imagery |
| [RFC-014](rfcs/RFC-014-automatic-artist-banner.md) | An automatic artist banner |
| [RFC-015](rfcs/RFC-015-artist-credit-splitting.md) | Splitting a joined artist credit |
| [RFC-016](rfcs/RFC-016-audio-quality-analysis.md) | The real quality of a file, measured from its audio |
| [RFC-017](rfcs/RFC-017-sign-in-hardening.md) | Hardening sign-in |
| [RFC-018](rfcs/RFC-018-incremental-scan.md) | An incremental scan |
| [RFC-019](rfcs/RFC-019-exposure-hardening.md) | Exposure hardening: first-run setup, browser headers, upgrade copies |
| [RFC-020](rfcs/RFC-020-playlists-and-shares.md) | Playlists and shares |
| [RFC-021](rfcs/RFC-021-library-lifecycle-and-scan.md) | Libraries and scanning: exclusions, links, missing tracks, watching |
| [RFC-022](rfcs/RFC-022-playback-and-web-client.md) | Playback and the web client: loudness, install, AAC |
| [RFC-023](rfcs/RFC-023-hardening.md) | Bounded reads, FFmpeg's protocols, the API reference, disabled accounts |
| [RFC-024](rfcs/RFC-024-lyrics-stats-and-mixes.md) | LRCLIB lyrics, listening statistics, mixes and local similar songs |
| [RFC-025](rfcs/RFC-025-smart-playlists-and-podcasts.md) | Smart playlists and podcasts |

## Plans and analyses

Thematic documents, kept current: each opens with the date and the `main`
revision of its survey, and a status line saying what has happened since.

| Document | Subject |
|---|---|
| [Metadata enrichment plan](plans/metadata-enrichment-plan.md) | The order in which RFC-011 was built, step by step |
| [Interface and missing features plan](plans/ui-and-missing-features-plan.md) | The interface requests and the features still missing, as issues |
| [Standalone download plan](plans/standalone-download-plan.md) | The server as a double-clickable archive with FFmpeg beside it: licences, laptop mode, signing, release jobs |
| [OpenSubsonic gap analysis](analyses/opensubsonic-gap-analysis.md) | The façade against the OpenSubsonic specification |
| [Web client gap analysis](analyses/web-client-gap-analysis.md) | The embedded web client against Navidrome's |
| [Desktop integration gap](analyses/desktop-v2-integration-gap.md) | The desktop app against server v2; closed, kept as a record |

## Releases

The notes published with each stable release, one per tag, in
[`releases/`](releases/): [2.0.0](releases/v2.0.0.md) and
[2.0.1](releases/v2.0.1.md). The release workflow refuses a stable tag that has
none. Every change, release by release, is in the [CHANGELOG](../CHANGELOG.md).
The screenshots the README shows are in [`screenshots/`](screenshots/).

## History

| Document | What it records |
|---|---|
| [M4 handoff](history/M4-handoff.md) | How milestones M0 to M6 closed. Dated, and in French on purpose. |

The session handoffs written between 2026-08-23 and 2026-09-15 were removed
once their content had moved into the guides, the plans and the issues; they
remain in the repository's history.
