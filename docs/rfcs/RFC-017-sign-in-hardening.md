# RFC-017 — Hardening sign-in: failed attempts, timing, and the cost of a guess

- **Status**: Proposed
- **Implemented by**: in progress: [#321](https://github.com/InstaZDLL/waveflow-server/pull/321)
  (this RFC). The *Status* field above never flips in this
  project; this line is the one that says what is running.
- **Date**: 2026-10-24
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md) (the credential
  model, the frozen Subsonic contract)

## Problem

Four independent audits of this server against Navidrome, Meelo, kima-hub and
Ampache (2026-10-24) reported the same weaknesses in how a password is
checked. Each was confirmed in the code:

1. **The native and web sign-ins are not throttled at all.** Both
   `POST /api/v2/auth/login` and `POST /api/v2/web/auth/login` call
   `Authentication::login` (`src/authentication.rs`), which checks the password
   with no limit on how often anyone may try.
2. **Unknown accounts answer faster.** An unknown or disabled username returns
   before any Argon2 work, while a real one costs a full verification. The
   difference in response time tells an attacker which usernames exist.
3. **Every guess costs the server about 19 MiB and a blocking thread.** Each
   verification runs Argon2id with the default parameters in `spawn_blocking`,
   and nothing bounds how many run at once. A burst of guesses is a memory and
   CPU exhaustion attack that needs no valid account.
4. **The Subsonic limit can lock the rightful owner out, and can be raced.**
   `src/subsonic/auth.rs` keys its window on the username (or the API key)
   alone and refuses before checking the password. Twenty wrong guesses a
   minute from anywhere therefore refuse the owner's own clients. The check
   and the record are also separated by database awaits, so twenty concurrent
   requests all pass the check before any of them is recorded.

## Decision 1 — one limiter, for every place a password is checked

A single in-process limiter, held on `AppState`, guards all three surfaces: the
native sign-in, the web sign-in and Subsonic authentication. It counts
**failures** per **(client address, account)** pair: twenty in a sliding
minute, the figure Subsonic already uses.

- **Keyed by the pair, never by the account alone.** An attacker elsewhere
  exhausts their own budget for that account, not the owner's. That closes the
  lockout of point 4. The cost is that an attacker spread over many addresses
  is slowed only by the work each guess costs (decision 3). For a self-hosted
  server that is the right trade: the alternative hands any stranger a switch
  that turns a user's clients off.
- **The account is what was typed**: the username for a password, or the
  API key for Subsonic's `apiKey`. It is stored as a hash, as today, so the
  limiter's memory never holds a key in clear.
- **The attempt is reserved before the work, not after.** A request takes a
  slot under the lock before anything is awaited. Twenty concurrent requests
  therefore take twenty slots, and the twenty-first is refused, which closes
  the race in point 4.
- **What keeps a slot, and what gives it back.** A request that cannot be a
  guess takes no slot: a malformed body, or a device name that fails
  validation, is refused before the reservation is made. Once taken, the slot
  is kept by default, through a guard that keeps it unless told otherwise. It
  is given back on two outcomes only:
  - a **success**;
  - an **outage before the password was compared**: the account lookup fails,
    or no verification permit is free (decision 3). Both answer `503`.

  A wrong password, an unknown or disabled account, and a request abandoned
  once its verification has started all keep the slot. That holds even when the
  client hangs up, since the verification goes on to completion whether anyone
  waits for it or not.
- **When the limit is reached, nothing is checked.** The answer is the
  surface's usual refusal, so the response never says whether the password
  was right:
  - `429` on the native and web sign-ins, with a `Retry-After` header;
  - Subsonic's error 40, unchanged, since the wire contract is frozen.
- **The table is bounded, and an active window is never evicted.** Each key is
  a client-chosen value, so evicting the oldest window when the table fills,
  as the Subsonic limiter does today, would let an attacker flush the window
  holding their own failures by sending a burst of fresh keys. So:
  - expired windows are pruned first;
  - when the table still holds 65,536 live windows, a **new** key is refused
    as throttled, while keys already in the table go on as before;
  - the limiter fails closed for strangers before it ever forgets a window that
    is counting.

  The limiter lives in memory, so a restart forgets it. That is acceptable: a
  restart is not something an attacker controls.

## Decision 2 — the client address

Nothing in the server knows the client's address today. The listener is served
with axum's `ConnectInfo`, so every request carries the peer's address.

Behind a reverse proxy, the peer is the proxy, and every client would share one
address. So **`WAVEFLOW_TRUSTED_PROXIES`**, a comma-separated list of addresses
or CIDR blocks, empty by default, names the proxies whose `X-Forwarded-For` is
believed. A request whose peer is not trusted is identified by its peer, and
its `X-Forwarded-For` is ignored: believing a header from anyone would let each
guess claim a fresh address.

When the peer is trusted, the header is read **from the right**:

- several `X-Forwarded-For` headers are joined in the order received, as one
  comma-separated list;
- walking from the right-most entry leftwards, every entry that is itself a
  trusted proxy is skipped, and the first one that is not is the client;
- an entry that does not parse as an IP address stops the walk, and the client
  is the last trusted hop reached, the peer if none. A malformed header
  therefore never yields an address the attacker chose;
- if every entry is a trusted proxy, the client is the left-most one.

**The contract with the operator.** Reading from the right is only sound if
each trusted proxy **appends** the address it received the connection from:
nginx's `$proxy_add_x_forwarded_for`, Caddy's and Traefik's defaults. Entries
to the left of the last proxy's own entry are whatever the client sent, and the
walk never reaches them while a non-trusted entry stands to their right.

A trusted proxy that forwards a client-supplied header without appending lets
the client choose its address. `docs/operations.md` says, beside the setting,
that only such appending proxies may be listed.

Without the setting, behind a proxy, every client shares a budget per account.
That is no worse than today's Subsonic limit, and `docs/operations.md` says to
set it.

## Decision 3 — every attempt costs the same, and the cost is bounded

- **The same Argon2 work for an account that does not exist.** An unknown
  username, or a disabled account, is verified against a fixed dummy Argon2
  hash with the same parameters, made once at startup. Only then is it refused,
  with the same `InvalidCredentials`. This removes the large signal, a whole
  Argon2 verification present or absent. It does not make the two paths equal
  to the microsecond: the account lookup, the branches and the error handling
  still differ slightly. The claim is limited to that. The PR measures the two
  paths in a test and asserts that their difference is small beside one
  verification.
  Subsonic compares a decrypted password, a cheap step, so its timing is not
  the issue there; only decision 1 applies to it.
- **At most a few verifications at once, and a bounded queue.**
  - **Running.** A semaphore admits `max(2, available CPUs)` Argon2
    verifications at a time. The permit moves into the blocking task, so it is
    held for exactly as long as the hashing runs, even when the request that
    started it has gone.
  - **Waiting.** At most four times as many requests may wait for a permit. A
    request arriving when the queue is full answers `503` at once, without
    waiting.
  - **Timing out.** A request that waits five seconds without a permit answers
    `503` as well.

  Neither `503` counts as a failure (decision 1). What this bounds is
  **Argon2's working memory**, about 19 MiB per running verification: its
  total no longer grows with the request rate. It does not bound the
  connections themselves, which are a matter for the listener and the reverse
  proxy.

## What does not change

- The responses of a successful sign-in, the token formats and the cookies.
- The Subsonic error codes and messages.
- `POST /api/v2/setup`, the first-boot admin creation, is out of scope; it is
  listed below.

## Logging

**Refusals are counted, not logged one by one.** A burst of guesses must not
become a burst of log lines.

- **Counted.** The limiter counts refusals per surface (native, web, Subsonic)
  and per reason (throttled, wrong credentials, queue full). The counts are
  plain atomics, so a burst costs nothing to record.
- **Logged once a minute at most.** At most once a minute, and only when a
  count has moved, one `warn` line gives the counts for that minute.
- **The first one immediately.** The first throttled refusal after a quiet
  minute is also logged at once, so an operator watching sees an attack begin.
- **Nothing per attempt above `debug`.**

The log volume is therefore bounded by the clock, not by the attacker.

No line ever carries the username, the address, the key or the password. The
`CLAUDE.md` rule against logging tokens and passwords has no exception for an
attack in progress.

## The order of the work

1. **This RFC.**
2. **One PR**, containing:
   - the limiter and its use on the three surfaces;
   - the client address and `WAVEFLOW_TRUSTED_PROXIES`, with its header walk;
   - the dummy hash, the semaphore and its bounded queue;
   - the aggregated refusal log;
   - tests for each rule, among them: a saturated table never drops a live
     window; an abandoned verification keeps its slot; a malformed or
     client-prefixed `X-Forwarded-For` never yields the client's chosen
     address;
   - `docs/operations.md`.

## What is still open

- **The first-boot admin.** `POST /api/v2/setup` creates the first account for
  whoever reaches it first; it checks only the `Origin` header. A one-time
  setup token printed in the log, or a compose file that binds to `127.0.0.1`
  by default, would close that window.
- **HTTP security headers** (`frame-ancestors`, `nosniff`,
  `Referrer-Policy: no-referrer` on `/share/`), reported by the same audits.
- **Second factor (TOTP)** on the web sign-in.
- **A per-address ceiling across accounts**, against password spraying from
  one place. It is left out because, behind an unconfigured proxy, it would
  let one client exhaust everyone's budget.
