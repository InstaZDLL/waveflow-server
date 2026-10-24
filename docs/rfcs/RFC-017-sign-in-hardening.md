# RFC-017 — Hardening sign-in: failed attempts, timing, and the cost of a guess

- **Status**: Proposed
- **Implemented by**: nothing yet. The *Status* field above never flips in this
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
  slot under the lock before anything is awaited, and gives it back on success.
  Twenty concurrent requests therefore take twenty slots, and the
  twenty-first is refused, which closes the race in point 4.
- **When the limit is reached, nothing is checked.** The answer is the
  surface's usual refusal, so the response never says whether the password
  was right:
  - `429` on the native and web sign-ins, with a `Retry-After` header;
  - Subsonic's error 40, unchanged, since the wire contract is frozen.
- **The table is bounded**, as the Subsonic one is now: past a fixed number of
  keys, the oldest window is dropped. The limiter lives in memory, so a restart
  forgets it. That is acceptable: a restart is not something an attacker
  controls.

## Decision 2 — the client address

Nothing in the server knows the client's address today. The listener is served
with axum's `ConnectInfo`, so every request carries the peer's address.

Behind a reverse proxy, the peer is the proxy, and every client would share one
address. So **`WAVEFLOW_TRUSTED_PROXIES`**, a comma-separated list of addresses
or CIDR blocks, empty by default, names the proxies whose `X-Forwarded-For` is
believed. The client is the right-most address in that header that is not
itself a trusted proxy. A request whose peer is not trusted is identified by
its peer, and its `X-Forwarded-For` is ignored: believing a header from anyone
would let each guess claim a fresh address.

Without the setting, behind a proxy, every client shares a budget per account.
That is no worse than today's Subsonic limit, and `docs/operations.md` says to
set it.

## Decision 3 — every attempt costs the same, and the cost is bounded

- **The same work for an account that does not exist.** An unknown username,
  or a disabled account, is verified against a fixed dummy Argon2 hash with
  the same parameters, made once at startup. Only then is it refused. The
  answer and its timing no longer depend on whether the account exists.
  Subsonic compares a decrypted password, a cheap step, so its timing is not
  the issue there; only decision 1 applies to it.
- **At most a few verifications at once.** A semaphore admits
  `max(2, available CPUs)` Argon2 verifications at a time. A sign-in that
  cannot get a permit within five seconds answers `503`, and does not count as
  a failure. Memory spent on guesses is therefore bounded by the semaphore and
  not by the request rate.

## What does not change

- The responses of a successful sign-in, the token formats and the cookies.
- The Subsonic error codes and messages.
- `POST /api/v2/setup`, the first-boot admin creation, is out of scope; it is
  listed below.

## Logging

A refused attempt logs at `warn` with its surface and the words "throttled"
or "unknown account". It never logs the username, the address, the key or
the password. The `CLAUDE.md` rule against logging tokens and passwords has no
exception for an attack in progress.

## The order of the work

1. **This RFC.**
2. **One PR**: the limiter and its use on the three surfaces, the client
   address and `WAVEFLOW_TRUSTED_PROXIES`, the dummy hash, the semaphore,
   tests for each rule, and `docs/operations.md`.

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
