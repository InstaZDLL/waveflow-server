# RFC-003 — WaveFlow Desktop user-data sync v2

- Status: accepted
- Date: 2026-08-09
- Scope: WaveFlow Server v2 M4 and the future Desktop remote-source adapter

## Decision

The server is authoritative for its catalogue. Desktop exposes it as a separate
remote source and synchronizes user-owned state only: playlists, favorites,
ratings, scrobbles/history, play queue and public shares. This protocol never
imports server tracks into the local catalogue and never guesses a local/server
track match. Reconciliation remains M5 and requires its own RFC.

All public IDs are UUIDs and all timestamps are Unix milliseconds. REST is the
durable source of truth. The WebSocket is only an edge-triggered notification
that a newer cursor may exist.

## Authentication

Desktop obtains a short-lived access token and rotating refresh token through
Authorization Code + PKCE. A mutation may send:

- `X-WaveFlow-Operation-Id: <uuid>` — stable ID for this logical mutation;
- `X-WaveFlow-Device-Id: <uuid>` — the non-revoked device created by login or
  the PKCE exchange.

The server rejects a device owned by another account. If no operation ID is
provided, the server generates one; clients that need retry safety must provide
one. An operation ID is unique per user and must never be reused for another
logical mutation.

## Bootstrap and incremental reads

`GET /api/v2/sync/snapshot` returns one writer-consistent representation:

```json
{
  "cursor": 42,
  "playlists": [],
  "favorites": [],
  "ratings": [],
  "queue": null,
  "history": [],
  "shares": []
}
```

The client atomically replaces its remote user-data projection, then continues
from `cursor`.

`GET /api/v2/sync/changes?after=<cursor>&limit=<1..500>` returns changes in
strict ascending cursor order. `next_cursor` is the last returned cursor, or
the supplied cursor for an empty page. While `has_more` is true, the client
immediately requests the next page.

Each change has `cursor`, `event_id`, `operation_id`, optional
`origin_device_id`, `entity_type`, `entity_id`, `action`, `payload` and
`changed_at`. Supported pairs are:

| Entity | Actions | Payload |
| --- | --- | --- |
| `playlist` | `upsert`, `delete` | id, name/comment/public when known, ordered `track_ids` |
| `favorite` | `upsert`, `delete` | `entity_type`, `entity_id`, `starred` |
| `rating` | `upsert`, `delete` | `entity_type`, `entity_id`, `rating` (0 means clear) |
| `scrobble` | `upsert`, `append` | `track_id`, `submission`, `played_at` |
| `queue` | `upsert` | ordered `track_ids`, current track, `position_ms`, client |
| `share` | `upsert`, `delete` | id and the changed non-secret share fields; bearer token and URL are never synchronized |

Unknown entity types, actions and payload fields must be ignored and retained
only if a client needs to relay diagnostic data. A client that cannot apply a
known event discards its local projection and fetches a fresh snapshot.

## Idempotency, acknowledgement and wake-up

The operation reservation, domain mutation and journal append commit in one
SQLite transaction behind the process-wide writer gate. Repeating the same
operation ID is recognized as already applied and never creates a second domain
row or journal event. Resource endpoints return the current representation when
it still exists; the journal receipt remains the durable proof of application.
The reservation stores a canonical fingerprint of the action, target resource
and normalized payload. Reusing an operation ID with a different fingerprint is
rejected as a conflict instead of being reported as a successful replay.

`PUT /api/v2/sync/ack` with `{ "device_id": "<uuid>", "cursor": 42 }`
records a monotonic per-device acknowledgement. A cursor below the stored ACK
does not move it backwards; a cursor beyond the user's latest event is rejected.
ACKs are observability and future-retention inputs, not a prerequisite for
reading later pages.

`GET /api/v2/sync/socket?after=<cursor>` upgrades to WebSocket and sends JSON
messages shaped as `{ "cursor": 43 }`. Authentication uses the same Bearer
header as REST. On every notice, reconnect, timeout or lag, the client asks
`/sync/changes`; it never treats socket delivery as state delivery.

## Cross-protocol convergence

Native, embedded-web and Subsonic mutations all call the same domain services.
Operations initiated by web/Subsonic receive server-generated operation IDs and
therefore appear in the same journal. Tenant filters are applied in repository
queries before data reaches any facade.

The journal is append-only in v2.0. Retention/compaction may be introduced only
with a snapshot floor that prevents an offline client from silently skipping
events.

## Expired cursors

`GET /sync/changes` refuses a cursor that precedes the oldest retained event
with **409** and `{"code": "cursor_expired"}`. The client discards its local
projection, takes a fresh `/sync/snapshot`, and resumes from its cursor.

The status is shared with idempotency conflicts, so **the code is what clients
must branch on**, not the status: `conflict` means mint a new operation id and
retry the mutation, `cursor_expired` means re-snapshot. Reacting to one as if it
were the other either loses a write or wipes a healthy projection.

Recovery is a **full snapshot**, never a resume from the surviving floor.
Retrying at `floor + 1` succeeds — a cursor at or above the floor is served
normally — but it silently skips whatever the compacted events carried, leaving
the projection permanently short with nothing to signal it. That a floor cursor
answers 200 describes a client that never fell behind; it is not a recovery
path.

The check is implemented and tested today even though it cannot fire: the
journal is append-only, so no cursor can fall below the floor. It is specified
now so clients write the recovery branch against a real contract rather than
discover it the day retention lands.
