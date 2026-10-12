# RFC-010 — External scrobbling

- **Status**: Proposed
- **Implemented by**:
  [#191](https://github.com/InstaZDLL/waveflow-server/pull/191), the durable
  half — the two tables, the queueing inside the transaction that writes
  `play_event`, the five verdicts, and the drain task; then
  [#192](https://github.com/InstaZDLL/waveflow-server/pull/192), the outgoing
  call — decision 10's bounded surface, the ListenBrainz adapter, and the
  timeout decision 6 promised to honour without the verdict being able to carry
  it.
  Then [#193](https://github.com/InstaZDLL/waveflow-server/pull/193), **the
  native routes and the CLI**: an account places and withdraws its own
  authorization, reads the state of its queue, and answers for the listens
  nobody knows arrived — decision 13 asked a person for that choice without any
  surface showing them what it was about. On the command line an operator places
  and withdraws an authorization and reads a queue's state — three gestures, for
  a server nobody has yet opened in a browser; answering for an uncertain listen
  remains a decision its author takes, and exists only on the HTTP surface.
  Then [#199](https://github.com/InstaZDLL/waveflow-server/pull/199), **decision
  13's fix**: the single retry is counted on `retried_at`, a column the entry
  carries, and not on the survival of the retry that consumed it — which
  retention takes away. The four readers that read the join now read the column,
  and databases already in service receive it filled.
  Then [#200](https://github.com/InstaZDLL/waveflow-server/pull/200), **the
  rest**: queue retention and its eighth background task; named destinations,
  with the URL fingerprint that makes their identity, the back-fill and the
  reconciliation at start-up; the Maloja adapter, where the envelope pours
  without losing a credit; and the Last.fm authorization journey, two routes and
  a table that lives a quarter of an hour.
  Finally [#201](https://github.com/InstaZDLL/waveflow-server/pull/201), **the
  surface that makes all of it reachable**: the web client's
  `/settings/scrobbling` screen, where authorizations are placed and withdrawn
  in pairs, the Last.fm journey opens, and the listens decision 13 refuses to
  settle are answered for. That path is not a choice: `lastfm_callback`
  redirects there, and as long as it did not exist a journey carried to its end
  landed on a "not found" page — the link made, and nothing to say so. Two
  server fixes came with it, both required by the screen: an `uncertain` entry
  now names its **instance** and not merely its recipient — without which
  decision 13 asks its question without saying which of the two profiles may
  hold the listen — and an inherited link the reconciliation breaks is given a
  name before being broken, instead of leaving the API with an empty instance no
  gesture could withdraw.
  The *Status* field above does not flip — it never flips in this project.
- **Date**: 2026-09-13
- **Revised**: 2026-09-13, after an external review. Decisions 2 to 6 changed,
  and each says what the earlier version asserted falsely rather than erasing
  it. A second pass the same day settled the three remaining questions:
  decisions 11 to 13.
  Then on 2026-09-14, after a second external review and a measurement: a
  retry's single chance is now counted on the entry (decision 13), a destination
  may exist in several named instances (decision 10), Last.fm gets an
  authorization protocol instead of a pasted secret (decision 11), and queue
  retention is settled. Those four are implemented the same day, by #199 and
  #200 above.
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md),
  [RFC-003](RFC-003-waveflow-sync-v2.md)
- **Closes**: the last of the five gaps against Navidrome that is noticed in
  daily use, and the one item of the web plan no batch carried

## Problem

A listen never leaves the server. `scrobble_with_context`
(`src/services/playback.rs`) writes a row into `play_event` and maintains
`now_playing`; `GET /api/v2/history` reads them back. That is all. Last.fm,
ListenBrainz and Maloja receive nothing.

It is the one of the five gaps against Navidrome whose absence is seen every
day: a listener who has kept a listening profile for ten years does not abandon
it because they changed server.

It is not, however, a feature one plugs in. **The server makes no outgoing call
today**, and making it do so changes its posture: a personal installation behind
a proxy becomes software that talks to third parties, with credentials, a queue
that survives restarts, and failures that are not its own. Every one of those
sentences is a decision, and that is why this RFC comes before the code.

> **Corrected on 2026-09-13, while writing
> [#192](https://github.com/InstaZDLL/waveflow-server/pull/192).** This RFC added
> "— `Cargo.toml` carries no HTTP client —" here and called the arrival of an
> outgoing dependency, further down, "the heaviest in consequences".
> `cargo tree -i reqwest` answers in three lines: **reqwest 0.12 arrives through
> `waveflow-core`, with rustls, and has been linked into this binary all
> along.** `Cargo.toml` *declared* none; the binary *linked* one. Declaring the
> dependency therefore added neither code nor a second TLS stack —
> `Cargo.lock` moved by one line — and the step was far lower than this
> paragraph dramatised.
>
> What stays entirely true, and was the real content of the worry: **the server
> talked to nobody**, and now it can. The posture changes; it was the dependency
> graph that did not.
>
> And the trap was real, somewhere other than where the RFC looked for it: Cargo
> unifies features, so declaring `reqwest` with its defaults would have switched
> on `default-tls` and brought a second TLS stack in **beside** rustls rather
> than in its place. It is the declaration copied from `waveflow-core` that
> avoids it.

## Decision 1 — one attachment point, the one where both surfaces converge

`scrobble_with_context` and nothing else. The native API and the Subsonic facade
both already arrive there: that is what M4 built, and wiring the transfer
anywhere else would duplicate it immediately.

**The queueing is written inside the transaction that writes `play_event`**,
under the same writer gate. A listen therefore cannot be transmitted without
having been recorded, nor recorded without being queued. Sending from the HTTP
handler, after the transaction, would leave exactly the window everything else
in this server works to close.

Idempotence comes with it: `claim_operation` already replays a repeated
operation by rolling the transaction back. A replayed listen writes no second
row, so it produces no second queueing either.

## Decision 2 — a durable queue, which freezes what was listened to

A `scrobble_outbox` table, drained by a background task shaped like the six
others — `spawn_upload_sweeper`, `spawn_canvas_sweeper`,
`spawn_artwork_sweeper` and their neighbours, all started in `src/main.rs`.

The alternative — calling the remote service during the request — is set aside
for three reasons, each sufficient on its own: a client's request would wait on
a third party whose latency and availability the server does not control; a
restart would lose whatever had not left; and a service silent for a day would
fail listens the server nevertheless recorded correctly.

**Every row carries an immutable envelope**: `played_at`, the title, the
credited artists, the album and its artist when they exist, the duration, and
the MusicBrainz recording identifier if it is known. The `play_event_id` stays,
as provenance. The link to the track is no longer used to read the metadata at
drain time.

**An earlier version of this decision said the opposite**: "no copy of the
track, the track can be re-read, and two models of one thing end up
contradicting each other". That argument holds against a *projection* of a live
row; it is false here, because the snapshot must precisely not follow the track.
Since [#186](https://github.com/InstaZDLL/waveflow-server/pull/186) a member
corrects title and artists, and a list correction rewrites `track_participant`:
re-reading at drain time would send what the track has become, not what was
heard. A frozen envelope tells the truth of a moment, which is exactly what a
listening history records.

That also answers a question the earlier version left open: a track deleted
between the listen and the send no longer empties the queue row.

## Decision 3 — listens are queued, and "now playing" never touches the request

`play_event.submission` already distinguishes the two, and the distinction is
the same at the recipients: `updateNowPlaying` at Last.fm, `playing_now` at
ListenBrainz, where it is explicitly temporary and records nothing.

**A submission is queued. A "now playing" is not.** It has value only while it
is true: retransmitting it ten minutes later would announce a track the listener
left long ago. Guaranteeing the delivery of stale information is worse than
losing it.

**But "unqueued" does not mean "in the request".** No HTTP call to a third party
leaves a client request's path, ever, for any reason. After the commit, the "now
playing" is dropped into a bounded in-memory channel the drain consumes. Channel
full, or a restart: the event is lost, and that is the intended behaviour.

## Decision 4 — two tiers of credentials, and the binary carries none

The precedent exists and it is good: the dedicated Subsonic password is
encrypted by `SecretBox` in ChaCha20-Poly1305 under the instance key.

But one tier is not enough, and the earlier version of this decision assumed it
was. ListenBrainz and Maloja are content with one token per person. Last.fm asks
for three things: an API key and a shared secret that identify *the
application*, then a session key that identifies *the account*. So:

- **At the deployment level**: the provider's API key and secret, placed by the
  operator. **Never embedded in the binary** — this repository is published
  under AGPL, and a secret in a public binary is not a secret. An operator who
  wants Last.fm declares their own application credentials.
- **At the account level**: the session key, or the personal token depending on
  the recipient, sealed under the instance key.

**Per account, not per library.** A listen belongs to a person; a per-library
setting would pose a question nobody wants — under which profile a listen counts
in a shared library.

**Never read back in the clear by the API.** As with API tokens, they are
replaced, not re-read. The backup rule stays what `CLAUDE.md` already says:
`data/waveflow.db` and `data/instance.key` go together, or those secrets are no
longer decryptable — and that is the intended behaviour.

### Unlinking and relinking: a generation, not a pair

`scrobble_link` carries an identifier of its own, and **`scrobble_outbox`
references that link**, not the `(account, destination)` pair.

Without it: twenty listens pending, I unlink my Last.fm account, I link
another — and the twenty leave for the second profile. The account and the
destination are the same; the authorization is not.

Unlinking disables the link and makes its pending rows end in a terminal state.
Relinking creates a new generation, which sees nothing of the old one. A request
already emitted, on the other hand, cannot be recalled: that is the limit of the
method, and it is named here rather than discovered.

## Decision 5 — the boundary: exactly once on our side, at most one ambiguous attempt on theirs

**This is the decision the earlier version got wrong, and it contradicted
itself.** It said both "a duplicate is worse than a loss" and "network error or
5xx: we retry". The two are incompatible for a POST with no idempotence key, and
neither `track.scrobble` at Last.fm nor `submit-listens` at ListenBrainz offers
one.

The case that settles it: the server emits the request, the recipient records
it, the connection drops before the response comes back. A network failure and a
success whose acknowledgement was lost are **indistinguishable**. Retrying
produces a second scrobble.

**The transactional queue gives atomicity between WaveFlow and its own queue. It
cannot give atomicity between WaveFlow and a third party.** That boundary is
real, no setting moves it, and this RFC recognises it instead of promising
beyond.

What is therefore guaranteed, and nothing more:

- **Exactly one queueing**, on the WaveFlow side. A row is unique per
  `(play_event, link)`, a constraint held by the schema and not only by the
  code.
- **At most one attempt for an ambiguous result**, on the recipient's side. We
  retry only when we **know** the request did not get through.

The philosophy does not change — **prefer a loss to a duplicate** — because a
lost listen is missing from a counter, while a listen sent twice damages a
public history a person has sometimes kept for years and cannot easily correct.

**Order is not guaranteed and does not have to be**: every submission carries
its own `played_at`, which all three recipients accept. Serialising the queue
per account would cost head-of-line blocking for a property nobody observes.

## Decision 6 — the verdict comes from the adapter, never from the HTTP code

The earlier version classified by HTTP status. That is wrong for at least two of
the three recipients: **Last.fm answers `200` carrying an application code** in
the body — invalid session, temporary unavailability, rate limit — and Maloja
reports its refusals in its JSON. Reading the status alone would take a failure
for a success.

Every provider adapter therefore returns **a verdict**, and the drain knows only
these five words:

| Verdict | Conduct |
| --- | --- |
| `Accepted` | the row leaves, it is finished |
| `Retryable` | growing backoff with jitter, a bounded number of attempts, `Retry-After` honoured when it is there |
| `AuthBroken` | the link is marked broken, the API says so to the account, and **its pending rows end in a terminal state** |
| `PermanentReject` | the row leaves, with a log line: no retry will correct a payload the recipient refuses |
| `Ambiguous` | **no automatic retry** — the terminal state `uncertain`, counted and readable, by decision 5 |

The drain therefore knows nothing of Last.fm, ListenBrainz or Maloja. That is
what allows a fourth to be added without touching the queue.

**A queue that does not empty is a defect**, not a state. After the bounded
attempts, the row is abandoned and counted, and that count is readable.

## Decision 7 — what the server does not send

- **Nothing retroactive on activation.** Connecting an account does not send its
  history: nobody wants to see ten years of listens surface on their profile at
  once, and a recipient reads that as abuse.
- **No track it cannot name.** With no title and no artist, the submission is
  unusable and skews the recipient's statistics.
- **Nothing from a share.** `/share/{token}` serves a visitor with no account;
  there is no profile to credit.

## Decision 8 — the Subsonic facade does not move

The contract is frozen for `v2.0-beta`. No new field, no new method: a Subsonic
client goes on calling `scrobble`, and what happens next is none of its
business. That is already the case for everything this facade ignores.

## Decision 9 — the configuration is a route, not an environment variable

`src/config.rs` carries what belongs to the deployment. A scrobbling link
belongs to an account, so it is placed through the native API — and through the
CLI for an operator preparing a server with no browser, as with the Subsonic
password.

What stays with the deployment: decision 4's application credentials, the
outgoing timeout, the attempt ceiling, the drain interval, the retention window,
and the destinations themselves — their name as much as their URL, several per
recipient if the operator wants.

## Decision 10 — the outgoing surface is bounded, and that is the decision that counts

This is the real risk of this RFC: a server that calls a URL is a server one can
make call a URL. Maloja and ListenBrainz are self-hosted, so there will be a URL
field, and it cannot be free.

- **HTTPS only**, except for a target the operator explicitly declares in the
  clear on their own network.
- **The base URL is the operator's setting**, never an account's. A member picks
  their destination from the ones the server knows, and does not describe it.
- **No redirect followed**, not even to the host requested. The first draft said
  "to another host", which is not what the code does: it sets `Policy::none()`
  and stops at the first. It was the text that promised less, and it was wrong
  to promise it — a `302` to the same host is still a path nobody chose, and the
  exchange that converts a Last.fm token into a session key would send an
  application key and a signature there.
- **Timeouts and response size bounded**, like every other I/O here.
- **Nothing from the response body is returned to the client**: it goes to the
  log, and what the API shows is a state, not an echo.

**One exception, and Last.fm imposes it.** The rule above sends what the
provider answered to the log, because an error body is what explains a failure.
`auth.getSession`'s response is not an error body: it contains the session key,
which is good until revoked. It therefore does not go to the log — neither whole
nor truncated. What is written there is a verdict and a normalised cause, as for
any other submission. The rule held as long as no response carried a credential;
this journey introduces one, and the rule narrows accordingly rather than letting
it through.

### Several instances, named by the operator

**Revised on 2026-09-14.** "A member picks their destination from the ones the
server knows" assumed a plural the setting did not provide: one URL field per
recipient, hence one Maloja instance per server. ListenBrainz copes — almost
everybody aims at the public instance — but Maloja is self-hosted by nature, and
on a family server everybody has their own. A single field forces sharing an
instance or giving up.

The operator therefore declares **named** destinations, several per recipient if
they want, and a link carries `(provider, destination)` instead of `provider`
alone. The account picks a name from the list the server publishes; it still
describes no URL, and decision 10's barrier does not move an inch.

**The destination enters the path too.** `PUT` and `DELETE` addressed
`/api/v2/scrobble-links/{provider}`, which names nothing precise the moment a
recipient has two instances — and `DELETE` has no body to name it in. Both
become `/api/v2/scrobble-links/{provider}/{destination}`, and the listing
publishes the pair. No implicit default: a path with no destination is refused
rather than attached to the first one around — the slide the next two paragraphs
forbid. The surface dates from yesterday and nobody has built on it yet, so this
contract is corrected now or carried for ever.

**What that pair moves in the code, said here so it is not found afterwards.**
Three places hold a link as identified by its recipient alone today, and none of
them is satisfied with one more column:

- `register_scrobble_target` files an adapter by `ScrobbleProvider`, so two
  Maloja instances would have only one, pointing at a single URL. The registry
  is filed by the pair, and the drain chooses by the pair.
- `unlink_scrobble_on` unlinks "this account's live link at this recipient" —
  called by `link_scrobble` before every new authorization. As it stands,
  linking `maloja/bob` would unlink `maloja/alice` and cancel her queue. The
  clause takes the destination, failing which the feature would destroy itself on
  its second use.
- The unique index `scrobble_link(user_id, provider) WHERE status <> 'unlinked'`
  simply forbids the case: it becomes `(user_id, provider, destination)`.

This is not implementation left open. It is the shape the pair imposes, and the
three hold together: correcting two out of three gives a server that accepts two
destinations and erases one.

**A destination name is written into a path without being encoded there.** Since
it becomes a URL segment, it is bounded to what travels through a path without
argument — ASCII letters, digits, `-`, `_` and `.`, at most sixty-four
characters — and `.` as well as `..` are refused, those two being navigation
instructions everywhere a path resolves. Refusing a name outside that at
start-up costs the operator a clear message; accepting it would cost a segment
that decodes differently depending on who reads it. The Last.fm journey's
randomness follows the same rule, for the same reason, and is drawn from a
cryptographic generator over at least a hundred and twenty-eight bits: it is the
only thing separating a legitimate return from a fabricated one.

**No table, no hot administration.** A destination stays a deployment setting,
not a row one adds while running. A table would require reconciling the
configuration and the database at every start-up, and inventing an answer for a
link whose destination has disappeared from one but not the other. Adding an
instance costs a restart, which a self-hosted server tolerates. The day WaveFlow
has an administration console, that column becomes a table for a reason one can
see.

**A destination withdrawn breaks the link, it does not slide another one in.**
If the configuration no longer names `maloja/alice` at start-up, the links that
aimed at it go `broken`, nothing leaves any more, and nothing is remapped to
another instance of the same recipient. Sliding `alice` onto `default` would
send one person's listens to another's profile — the substitution decision 4
builds a whole tier of credentials to prevent. The operator puts the destination
back, or the person unlinks and relinks.

**And its pending rows end, they do not wait.** A `broken` link whose queue
stays `pending` is a queue nothing will ever empty: `pending` escapes retention,
so the table keeps those rows for ever, and `oldest_pending_at` holds the link
`degraded` for ever — a third instance of the defect this revision already
corrects twice. They therefore move to a terminal state, as unlinking already
does in decision 4: same situation, same rule, and retention can then do its
work.

**And this happens before the drain starts.** The order is half the rule:
reconciling the links while the background task is already running leaves a
window, short and sufficient, in which listens leave for yesterday's
destination. `initialize` therefore compares the identities and ends what has to
end under the writer gate, and `spawn_scrobble_drain` only begins afterwards.
Nothing else needs guarding: once the link is `broken`, queueing writes nothing
more for it.

**A name is not an identity: the URL is part of it.** Withdrawing
`maloja/alice` breaks its links, but giving it another URL would send them
elsewhere with nothing changing name — the same substitution, through the next
door, and the easiest one to commit since it looks like a configuration fix. A
link therefore keeps enough to recognise the destination it aimed at, the URL's
fingerprint included; if that no longer matches at start-up, it goes `broken` as
if the name had disappeared.

**What that fingerprint is, said before it is written.** It covers the
*normalised* URL, not the string the operator typed: without that,
`https://maloja.example` and `https://maloja.example/` would break every link on
a server at the first restart after an editor added a slash.

**And that normalisation is not the public URL's.** `normalize_public_url`
returns an origin — scheme, host, port — and throws the path away, because an
origin is what shares need. A destination is not: `validate_destination` accepts
a path, so `https://host/tenant-a` and `https://host/tenant-b` are two
destinations, and reducing them to their origin would give them the same
fingerprint. The identity guard would then let through exactly the move it
exists to see — from one tenant to another on the same machine, which is the
likeliest form such a change can take.

A canonicalisation of its own is therefore needed for destinations: scheme,
host, port and **path** serialised unambiguously, the query string and fragment
refused as they already are, and the trailing slash settled rather than left to
the reader — `https://host/maloja` and `https://host/maloja/` are **the same**
destination, because an operator who pastes either wants the same machine, and a
link broken by a slash added while editing a file would be a punishment for
nothing. The canonical form is the one without a trailing slash, the root aside,
where there is no choice to make. The same normalisation serves the back-fill,
the reconciliation and the retry guard: three readings of one fact, which are
only worth anything if they compute it the same way — and the two writings of
one URL are what a test holds together. Comparing fingerprints rather than URLs
is not a precaution against an attacker — nobody hostile writes this
configuration — but the way to have only one thing to compare.

This is decision 4 applied one tier up. There, unlinking then relinking creates
a generation that inherits nothing, because the account and the destination can
be the same while the authorization is not. Here, the name can be the same while
the machine at the end is not, and a pending queue must not discover the
difference by crossing it. Correcting a typo in a URL then costs a relink — the
price of a distinction no heuristic can make for the operator.

## What this RFC changes elsewhere

- **An outgoing dependency enters `Cargo.toml`** for the first time. It is the
  heaviest point in consequences, and it deserves saying rather than being
  discovered in a `cargo tree`.
- **Two tables**: `scrobble_link` and `scrobble_outbox`, as dated migrations, as
  always.
- **A seventh background task**, started in `src/main.rs` with the six others —
  then an eighth, for the purge, when retention is written.
- **More columns, in dated migrations like the tables.** On `scrobble_outbox`,
  `retried_at` — decision 13's invariant cannot be inferred from a join the
  purge unties — and the instant of the move to a terminal state that retention
  counts from. On `scrobble_link`, the destination's name and its URL's
  fingerprint.

  **And `retried_at` is filled too, on databases already in service.** Retried
  entries have existed since [#191](https://github.com/InstaZDLL/waveflow-server/pull/191),
  recognisable by the join this revision abandons. Adding the column empty would
  make all of them answerable a second time: the migration would reintroduce, on
  the only servers already running, exactly the defect it has just corrected. It
  therefore reads `retry_of` one last time to write the column, and takes as the
  instant the one when the retry row was created, which is the moment the single
  chance was consumed. That one holds in SQL — no configuration enters the
  computation — and a test on a database populated before the column is what
  proves it, not a re-reading.
- **And a back-fill, which is the part one forgets.** The links already written
  carry neither name nor fingerprint, and there is only one destination per
  recipient at the time this revision is written: they receive that one, with the
  fingerprint of the URL configured then. Leaving a null value in place would
  amount to treating every old link as a destination that had disappeared, and
  breaking at the next start-up links nothing had moved.

  **The terminal instant is back-filled too**, and on the same databases already
  in service: the `sent`, `rejected`, `abandoned`, `cancelled` and `discarded`
  rows written before the column carry none, and the purge would not know from
  when to count them. They receive their `updated_at`, which is the write that
  made them terminal — nothing has touched them since. The `uncertain` ones
  receive none: they are not purged, and giving them one would invite somebody to
  use it one day to count them. This is tested like the rest, on a database
  populated before the column: every terminal state back-filled, the `uncertain`
  ones left empty, and a terminal instant a later write does not move again.

  **Why `updated_at` does the job here**, when it would not for what comes next.
  All eight `UPDATE`s `scrobble_outbox` knows require a non-terminal starting
  state — `pending`, `sending` or `uncertain` — in their `WHERE`. Nothing writes
  to a row that is already `sent`, `rejected`, `abandoned`, `cancelled` or
  `discarded`: its last write *is* its transition, and there is therefore no
  historical row whose instant would be undeterminable. That is a property of
  today's code, not a guarantee of the schema, and it is exactly why the column
  exists for what comes next rather than going on reading `updated_at`: the first
  `UPDATE` written without a state guard would make it fall without a sound.

  **This back-fill asserts, it does not verify.** It cannot: nothing in the
  database says which URL was configured yesterday, so writing the current
  fingerprint amounts to declaring that today's destination is indeed the one
  those links aimed at. That is true as long as there is only one possible
  answer, and it is the case as this is written. If inherited links exist at a
  recipient and the configuration already declares several for it, there is no
  answer: the server refuses to start rather than choose one, because being
  wrong here would send pending listens to the wrong profile with nothing
  reporting it. The way out needs no new mechanism: starting once with a single
  destination per recipient — the one those links aimed at — fills the columns,
  and the others are added at the next start-up. The back-fill happens only
  once; what comes after goes through the ordinary reconciliation.

  **None of this concerns a fresh installation.** The refusal is about the
  ambiguity of a back-fill, and with no inherited link there is nothing to
  back-fill: an empty database starts with as many destinations as the operator
  declares, from the first time. Not saying so would have made the feature
  unreachable for those who have nothing to migrate — a guard rail that closes
  the door it protects.

  **And if a recipient has links while no destination is declared for it** — a
  URL emptied out of the configuration, which is already a way of disabling
  it — there is nothing to write: those links are treated like those of a
  destination that has disappeared, broken and their queue ended, rather than
  left with two empty columns the next reconciliation would not know how to
  read.

  **A deployment consequence, said rather than kept quiet.** Whoever changes a
  destination's URL *then* updates will see the back-fill endorse that change:
  the fingerprint written will be the new one, and the pending listens will leave
  for the new machine without the identity guard, which never knew the old one,
  having anything to report. That is the one moment of the cycle where that guard
  cannot play, and it is in its nature: it compares against what has been
  written, and here nothing has been. The order to keep is therefore the
  reverse — update first, change the URL after, and the guard will do its work.
  An operator who has already done the other unlinks before updating.

  **SQL cannot do it.** `Database::migrate` has only the database; the
  fingerprint is computed over a URL that comes from `Config::from_env`, which
  the migration does not see. So the migration adds the columns, nullable, and
  `initialize` fills them — in the same place and under the same gate as the
  reconciliation described above, of which this back-fill is the first turn: fill
  then compare, once, before `spawn_scrobble_drain` leaves.
- **Documentation**: a section of the API guide, and a line in
  `docs/web-client-gap-analysis.md`, whose point 15 is waiting for this one.

## Decision 11 — the order of the recipients, and one listen per request

**ListenBrainz, then Maloja, then Last.fm.**

ListenBrainz marries the architecture above without asking anything of it: one
token per person, JSON, a permanent submission and a `playing_now` its
documentation explicitly gives as temporary — decision 3 exactly. It is
self-hosted, so it also tests decision 10 from the first day.

Maloja comes next: its native API directly accepts a list of artists, an album
artist, a duration and a timestamp, so decision 2's envelope pours in without
loss.

Last.fm last, **because it is the one that tests the abstraction most**: a
signature, application credentials on top of the account's session, and
application errors under a `200`. An adapter written for it first would have
leaked its particularities into the drain; written third, it meets a boundary
already held by two others.

### One listen, one request

Last.fm accepts up to fifty scrobbles per call. **We do not take advantage of
it, not at the start.** A request of fifty that comes back `Ambiguous` makes
fifty listens uncertain at once, and decision 5 then forbids retrying: the batch
turns one possible loss into fifty probable ones.

One queue row, one request. Grouping becomes a decision again the day
throughput is a measured problem, and on that day it will be necessary to say
what an ambiguous batch becomes — which this RFC does not have to settle for a
personal server.

### Last.fm is not pasted in, it authorizes

**Added on 2026-09-14.** All of decision 9's surface holds in one gesture:
presenting a secret already in hand to
`PUT /api/v2/scrobble-links/{provider}/{destination}`. Last.fm does not issue
one. A round trip through the person's browser is needed — a request token, an
authorization at their site, then the conversion of the token into a session
key, which lasts until revoked.

**The server carries that journey.** The other way — "get a session key with a
third-party tool, then paste it" — was tempting because it costs nothing: that
is also what disqualifies it. It takes a single destination out of the model the
other two live in, and makes the person pay for a provider's peculiarity. It
remains good for diagnosis, not as the normal path.

Two routes, and a temporary state that does not enter `scrobble_outbox`:

- `POST /api/v2/scrobble-links/lastfm/authorize/{destination}` opens a state
  bound to the account and returns the URL to send the person to: `/api/auth` at
  Last.fm, carrying the application key and a `cb` naming the route below.
- `GET /api/v2/scrobble-links/lastfm/callback/{state}` receives the `token`
  Last.fm adds there, verifies the state its path carries, calls
  `auth.getSession` to exchange the token for the session key, seals it and
  creates the link.

**The destination is named on the way out and found again on the way back.**
Last.fm having only one instance, it would be tempting to leave it implied — but
this section has just refused every implicit default, and an exception for one
recipient would be the first step towards the slide it forbids. The name is
therefore asked for, checked against the declared destinations, and filed in the
state: that is what the return re-reads, rather than guessing one. The day
somebody declares two Last.fms — a family account and their own — nothing will
have to change.

**And the return re-checks the destination.** A quarter of an hour separates the
outward journey from the return, and a server can restart in between — that is
precisely what the state lives in the database to survive. It therefore also
carries the destination's fingerprint as of the outward journey, and the return
compares it with the one after a restart before calling `auth.getSession`:
destination gone or moved, the return is refused and nothing is created. Without
that, the journey would manufacture a link to a machine the person did not
choose — which the identity guard prevents everywhere else, and which would be
worse here since the link would be born already wrong.

**The literal comes before the parameter, and that is not a style
preference.** If the route were written `…/lastfm/{destination}/authorize` — and
that is precisely why it is not — a destination named `callback` would produce
`…/lastfm/callback/authorize`, which the return `…/lastfm/callback/{state}`
claims just as much — two patterns of the same shape, and a router that has to
choose. Reserving the word would work; moving it removes the question. A list of
forbidden names is a thing one has to remember to keep up to date, and it is
forgotten at the first segment one adds.

**This is the web journey, and it does not call `auth.getToken`.** That method
belongs to the desktop-application journey, where the token is requested before
the page is opened; here it is Last.fm that hands it over, on the return. The two
resemble each other enough to be mixed up — the first draft of this paragraph
mixed them up — and an implementation that requested a token and then received
another would be working with the wrong one.

The state is single-use and **expires in ten to fifteen minutes**, so well before
the sixty Last.fm grants its token: we are the ones who refuse first, and never a
stale token that surprises us. It carries enough to recognise itself without
guessing anything: an account, a random value, a deadline.

**The random value travels in the path, and not in a query string.** Last.fm
announces that it hands the token over by appending `/?token=…` to the callback,
which says nothing about what it would do with a callback already carrying a
`?`. The `cb` therefore names `…/callback/{state}`, which depends on no
assumption about that concatenation.

**What the return refuses before calling anything at all.** A `token` that is
absent, empty, or present twice; and the case where Last.fm comes back saying it
failed. In each, nothing is exchanged and no link is created — the call to
`auth.getSession` comes after those refusals, not before. The repeated case
deserves naming: `?token=a&token=b` leaves an extractor to choose, and a journey
whose outcome depends on which duplicate a reader keeps is not a journey. The
refusal logs no more than the success.

**The trailing slash is not a drafting detail.** Last.fm announces that it
appends `/?token=…`, so what the browser will request is `…/callback/{state}/`
and not `…/callback/{state}`. `axum` does not normalise trailing slashes: a route
declared without it would not answer, and the journey would fail at its last
step, for everybody, over one character. The route therefore accepts both forms,
and a test holds it — not an attentive re-reading.

**And the state hangs on two things, not one.** Binding it to the account alone
would be enough if the return URL never left the browser that requested it — but
it goes through Last.fm, and a URL that travels ends up in a referrer or a
history. Whoever obtained it could finish the journey with *their* token, and
somebody else's account would start scrobbling onto their profile. The state is
therefore paired with the cookie described below, the return demands both, and it
is consumed before the token exchange rather than after the link is created:
what has served once cannot serve again, even if the first attempt fails further
on.

**That token arrives in a URL, and a URL is kept.** It is worth sixty minutes
and worth a profile: it has no business in a log, a browser history or a
referrer. Three consequences for this route, none optional. Its path joins the
prefixes `trace_path` redacts, alongside stream tickets and share tokens, which
covers the `{state}`; the `token` itself is in the query string, which no trace
in this server keeps anyway. `CLAUDE.md`'s rule suffers no exception for an
hour-long secret. The response carries `Cache-Control: no-store` and
`Referrer-Policy: no-referrer`, without which the token would travel to the next
page in a header nobody re-reads. And it redirects immediately to an address
with no token, so that it is that one the history keeps — the page the person
lands on has no need to know more than "it is linked".

**And the proxy's access log is not ours.** This server keeps no query string,
but a reverse proxy writes one by default, and the token would end up there
whole. An operator who puts one in front removes or masks the query string on
that path — without removing it from the forwarded request, which the handler
reads. That cannot be verified from here, like the registration of the callback;
it is said.

**The states live in the database and are purged.** Keeping them in memory would
lose them on a restart, in the middle of the one journey that cannot bear being
resumed. A tiny table, then, with a deadline — and retention's purge task passes
through it too, since it will exist anyway. It applies each state's own deadline
there and nothing else: the thirty-day window governs the queue, not a journey
that lasts a quarter of an hour, and borrowing the wrong one of the two would
keep a state long after it should have died. A revision that settles a table's
endless growth would look poor introducing another.

**The return destination is derived from `WAVEFLOW_PUBLIC_URL`**, which already
states the server's external origin for shares. One more setting for the same
thing would be a second truth to keep in agreement with the first.

**The return does not arrive with a bearer token, and not with the ordinary
session either.** It is a browser coming back from Last.fm, not a client calling
the API: no `Authorization` header accompanies it. And the web session is no
more sufficient, which the first draft of this paragraph wrongly asserted — this
server's cookies are all `SameSite=Strict` (`src/api/web_session.rs`), so a
return coming from another site carries none. The journey would have failed for
everybody, at its last step, for a reason invisible on reading.

What authorizes the return is therefore a **cookie of this journey's own**, set
by `authorize` and expected by it alone: `HttpOnly`, `Secure`, `Path` reduced to
the return's path, and `SameSite=Lax`, which is exactly what is needed — a
top-level `GET` navigation carries it, a background request from another site
does not. It is worth what the state is worth, lasts as little, and is thrown
away with it. The session cookies do not move: `Strict` is the right setting for
them, and this exception does not concern them.

**One journey at a time.** The cookie carries a fixed name, so opening a second
replaces the first: the tab left behind will no longer be able to conclude, and
its state will expire on its own. That is the limit kept rather than
corrected — linking one's account to a service is a gesture one makes once, and
a cookie per state would trade that small surprise for a mechanism nobody needs.
What must not happen is a return finding a cookie that does not correspond to it
and accepting it anyway: the two are verified together, or the return is refused.

**What RFC-002 requires, the return answers differently.** The rule is that a
route cannot exist without saying which `Access` it requires. `authorize`
answers like the others: the current account, and `Access::Write`, since placing
an authorization is a mutation. The return requires none, and that is not an
oversight: it is not called by a client but by the browser of a person returning
from a journey they have just opened. What stands in for proof is narrower than
an `Access::Write` — a single-use random value, a deadline of a few minutes, a
cookie that is only good for that path, and all three must agree. The exception
is named here rather than discovered in a missing `authenticated`.

**And this route wants `https`.** Decision 10 tolerates the clear towards a
target the operator declares on their own network; that way out does not apply
here, since the trip that brings the token back starts from the internet and goes
through a person's browser. A `WAVEFLOW_PUBLIC_URL` on `http` must therefore
leave Last.fm unavailable, exactly as an absent value does, and for a reason the
destination listing will state just as clearly.

That is a requirement to write, not an observed state:
`normalize_public_url` accepts `http` as well as `https` today, and must go on
doing so — the public URL also serves shares, where the clear on a private
network remains a legitimate operator choice. The condition belongs to
activating Last.fm, not to the setting.

**And that URL is registered with them.** A Last.fm application account carries a
callback, which the `cb` parameter serves to replace for a given authorization;
their documentation does not say whether that value is checked against the
registered domain. The operator therefore registers the URL their
`WAVEFLOW_PUBLIC_URL` produces, rather than betting on a tolerance nobody has
promised them. It is a deployment prerequisite on the same footing as the
application key, and it lives in the same place — nothing here can verify it
from this side.

It follows that **Last.fm is unavailable until `WAVEFLOW_PUBLIC_URL` is
configured** — the server already warns about that at start-up. That is not a
defect to work around but a condition to state: the destination listing
announces it, with its reason, rather than letting a link fail later with no
explanation.

**No browserless journey in this batch.** Last.fm documents one for desktop
applications, and it may come. Two journeys written together is two chances to
get the temporary state wrong, and the CLI is here only for the operator
preparing a server.

### It came, on 2026-09-15, and it has no temporary state

The deferral above rested on a named fear: *two chances to get the temporary
state wrong*. This one has none. The request token belongs to Last.fm, it
expires on their clock, and **it is the person who carries it** from one command
to the other. No table, no cookie, no deadline of ours. There were therefore not
two states to hold but one — and that is what allowed it to be written second
without replaying the fear.

Two commands, where the web has two routes:

- `waveflow-server scrobble authorize --actor … --username … --destination …`
  calls `auth.getToken`, prints the address to open and the token to bring back.
  **It writes nothing**: an authorization nobody finishes costs the account
  nothing, and no row lies around for a purge to pick up.
- `waveflow-server scrobble exchange --actor … --username … --destination …`
  exchanges the approved token for the session key and creates the link.

`--actor` is the administrator running it, `--username` the account that will be
linked: both, as everywhere in this CLI, because an operator prepares a server
for people other than themselves.

**It does not require `WAVEFLOW_PUBLIC_URL`**, and that is its reason for being.
The web journey requires it because Last.fm has to bring a browser back to an
address this server answers; here nothing comes back. The only condition is
therefore an application — which makes this the journey of a server with no
public front, exactly the machine where the CLI is already the only gesture
available.

**The token goes through `WAVEFLOW_LASTFM_TOKEN`, not through `argv`** — a name
`--token-env` allows changing, like `link`'s secret and for the same reason: a
shell history and a process list can be read. This one deserves that care as
much as a session key — whoever exchanges it first is the one the session ends
up belonging to.

**And `authorize` does not print a line ready to paste.** The first draft
displayed `WAVEFLOW_LASTFM_TOKEN=… waveflow-server scrobble exchange …`, ready
to copy — hence ready to land in the history the variable exists to avoid. The
token is printed on its own, the command names the variable, and **how it is
filled belongs to the operator and their shell**: a silent read, a secret
manager, a file sourced then deleted. This server runs on three platforms whose
answers differ, and it does not have to choose for them. The
[API guide](../api-v2-guide.md#external-scrobbling) shows one per shell, as an
example and not as a prescription.

**And `auth.getToken` does not enter the web journey for all that.** The
paragraph above said the two resemble each other enough to be mixed up; now that
both exist, that sentence stops being a warning and becomes checkable. The test
double counts token requests, and the web journey makes **zero** — a server that
minted its own and then received another would be working with the wrong one, and
that is now an assertion rather than an intention.

## Decision 12 — what the API shows: counters, never an echo

A link exposes its state and the shape of its queue, aggregated:

- `healthy`, `degraded`, `broken`;
- how many rows are waiting, how many are retrying, how many are `uncertain`;
- how long the oldest has been waiting, and when the last success was;
- possibly a last normalised cause — `rate_limited`, `auth_broken`.

**`healthy` cannot be allowed to mean "I still have a token".** A valid link
with three thousand listens pending for six hours is broken, and that is
precisely the silent failure a durable queue exists to make visible. Hence
`degraded`: the link answers, but the queue does not empty — an uncertain entry
waiting for a person to answer for it, or an entry pending for longer than the
threshold. `broken` stays reserved for `AuthBroken` and for an unusable
configuration.

**Corrected on 2026-09-14:** this sentence said "retries, uncertain entries, or
a wait that is too old", and `link_health` does not look at retries. It was
right not to — a row that retries is a queue that works, and counting it would
degrade every link that had met a passing failure. It was the text that promised
crookedly, from the start. The order of the three questions lives in
`link_health` and is not copied here: two writings of the same threshold would
diverge, and it is the code the API returns.

**Removed on 2026-09-14:** a revision of this document claimed here that "when
the last success was" was held by nothing, and that a column on `scrobble_link`
written at the moment of success would be needed one day. That column has existed
since [#191](https://github.com/InstaZDLL/waveflow-server/pull/191):
`scrobble_link.last_success_at`, written in the very transaction that settles a
row, published by `scrobble_links`. The shape prescribed was already the shape in
place.

The mistake is worth keeping because it has a clear cause: `link_health` does
not take that field, and I concluded from that that it did not exist. It does not
take it because health does not depend on it — a link that has never sent
anything is not sick — and the field travels beside it, in `ScrobbleLinkState`.
**Reading a function's signature does not say what a structure carries.**

**Never the envelope's content, never the provider's raw response.** Decision 10
already forbids the echo; this is its corollary on the reading side. What comes
out is a count, not a title nor an error body.

**And this does not travel through synchronization.** It is the server's
operational state, not user data to replicate to the desktop:
[RFC-003](RFC-003-waveflow-sync-v2.md) does not have to carry it, and a native
read of the link is enough. Adding it would make a counter that describes one
machine travel to every device on every wake-up.

## Decision 13 — `uncertain` is discarded or retried, and the duplicate is chosen

An `uncertain` entry is terminal for the server. The person has two gestures:
**`discard`** and **`retry`**.

It is the only form of duplicate this RFC accepts, because it is chosen. The
gesture therefore carries its warning: *the recipient may already have recorded
this listen; retrying may count it twice.* A person who cares about their history
will sometimes prefer the hole, and that is their judgement, not the server's.

**Retrying is not reactivating.** The ambiguous attempt stays in the history as
it was; the new one is added to it, marked as asked for by the person. Erasing
the first would make the only trace that explains why a duplicate exists at the
recipient lie.

**One entry at a time, and no "retry everything".** A button that relaunches
three hundred uncertain entries turns a conscious decision into an accident. It
can be added later, if somebody asks for it knowing what they are asking for.

### The single chance is counted on the entry, not on its descendants

**Revised on 2026-09-14.** The first writing inferred "already retried" from a
join: an entry was still answerable as long as no other row named it through
`retry_of`. That is false as soon as a row can disappear, and the retention
below makes rows disappear exactly — the retry ends `sent`, hence purgeable, and
the original then becomes answerable again. Measured rather than feared: with
the retry deleted, the entry reappears in `uncertain_scrobbles`,
`retry_uncertain_scrobble` accepts it a second time, and the link's `uncertain`
counter puts it back to `degraded` for a listen already answered for.

The single chance therefore becomes a fact the entry carries itself:
**`retried_at`**, null until it is not. The retry is written in one transaction,
the UPDATE first:

```sql
UPDATE scrobble_outbox SET retried_at = ?
 WHERE id = ? AND state = 'uncertain' AND retried_at IS NULL;
-- exactly one row modified, or the gesture is refused
INSERT INTO scrobble_outbox (..., retry_of, ...) VALUES (..., ?, ...);
```

The order matters: it is the conditional UPDATE that arbitrates, and two
simultaneous requests cannot consume the same chance twice. `retry_of` and its
unique index stay — they state the provenance, and refuse the insertion as a
second rampart — but nothing observable depends any more on the survival of the
row they name.

**A retry also checks where it would go.** `retry_uncertain_scrobble` already
requires an `active` link — a listen queued under a withdrawn authorization
would otherwise leave for the next one's profile, which decision 4 prevents. The
identity rule above adds the second half: the URL's fingerprint must still
match. Without it, the most dangerous entry there is — a listen the person
accepts risking twice — would be the one leaving for a machine nobody chose. The
check precedes the UPDATE, in the same transaction as it.

**No `resolution` state beside `state`.** Discarding already writes
`state = 'discarded'` on the entry: an enumeration that said `discarded` again
would put two sources of truth on the same fact. Only the retry leaves the entry
`uncertain`, and only it needs noting.

**And everything that read the join now reads the column.** This paragraph first
said *three* places — the list of uncertain entries, the link's counter and the
refusal of a second retry. There are **four**: the refusal of a *discard* too,
`discard_uncertain_scrobble`, which has carried the same exclusion since
[#193](https://github.com/InstaZDLL/waveflow-server/pull/193). The count came
from a re-reading of the decision rather than from the file; it is the file that
is right. All four move to `retried_at IS NULL`. The counter above all:
`link_health` returns `degraded` as soon as an uncertain entry is counted, so
forgetting a single one of those four places gives a link permanently in trouble
over a listen its person has already answered for — the failure decision 12
wants to make visible, turned into a permanent false alarm. Each of the four is
tested separately, by an inversion that puts only that one back on the join and
makes its single assertion fall.

## Queue retention

**Settled on 2026-09-14.** The question was not posed by the first version of
this RFC: the queue is never purged, so the table grows by one row per listen
per destination, endlessly, at an active listener's. That did not hold
[#191](https://github.com/InstaZDLL/waveflow-server/pull/191) back, and for
itself calls only for a deployment setting and a purge task, on the pattern of
[RFC-007](RFC-007-library-event-stream.md) and the eighth background task. The
columns this revision adds come from elsewhere, and are listed further up.

**Which instant we purge on.** Neither `created_at`, which dates the queueing
and would make a row leave thirty days after being written whatever the date it
finished, nor an `updated_at` nothing guarantees will not move again. A row keeps
the instant it became terminal, once, and the purge counts from there. That is
the kind of property that gets lost between an RFC and one more `UPDATE`, so it
is written down: that instant is not rewritten.

- **Thirty days for the ordinary terminal states** — `sent`, `rejected`,
  `abandoned`, `cancelled`, `discarded`. Enough to understand a past failure,
  too short for the queue to become a second listening history. It is a
  setting's default, not a carved duration: whoever diagnoses a three-month
  failure lengthens it, as they set RFC-007's window.
- **`uncertain` is kept indefinitely**, whether it carries `retried_at` or not.
  An entry with no answer is waiting for a person, and taking it away after a
  month would be deciding in their place. An answered entry could leave later;
  this is not the moment to decide that, because `retry_of` is declared
  `ON DELETE SET NULL` and the uniqueness index `(play_event_id, link_id)` only
  holds `WHERE retry_of IS NULL` — deleting the original un-nulls the retry and
  makes it fall back under that index. That is testable, and the gain counts in
  kilobytes.
- **No floor**, unlike RFC-007. There the stream serves a synchronization
  restart and cutting the head off a quiet library would send somebody back to
  the snapshot. Here nobody resumes a cursor: the outbox is a means of delivery,
  not a source of truth. What the link publishes — counters and health — is read
  off the live rows, `pending` and `uncertain`, which no purge touches.
- **What the link publishes depends on no purgeable row**, and that is already
  true: the counters and the oldest wait are read off `pending` and `uncertain`,
  which no purge touches, and `last_success_at` is a column of `scrobble_link`
  written at the moment of success. A revision of this document asserted the
  opposite here — that this last field did not exist and would be computed as
  `MAX(updated_at)` over `sent` rows — which would have been a real defect if it
  had been true. It was not: the column has been there since #191. What remains
  of the warning, and is worth keeping: nothing a link publishes must come to
  depend on the rows the purge takes away.

**What that bound does not bound, and why it is accepted.** A destination that
answers ambiguously on every send produces one `uncertain` per listen, and none
of them leaves: the table then grows as fast as the listener listens, exactly
what retention corrects elsewhere. The limit is real and it is kept as it is,
because the alternative requires settling what an entry becomes when nobody
answers — and decision 13 precisely refuses to answer in somebody's place. What
makes the case bearable is elsewhere: a link in that state is `degraded` from the
first ambiguity, so the failure is visible before the volume counts. If it turns
out to be visible without being looked at, the question will come back, and it
will come back with a measurement rather than a fear.

## What is still open

No architectural decision any more, and no retention question. What remains
belongs to the implementation: the exact shape of the JSON, and the threshold
beyond which a queue that does not advance becomes `degraded` — that one already
lives in `link_health`.

**The paths written above, on the other hand, commit.** The first version filed
"the exact name of the routes" among the details, which was true as long as no
route was decided. Decisions 10 and 11 now name some, and they do so because the
shape *is* the decision: a destination in the path rather than implied, a random
value in a segment rather than in a query string. Renaming them is possible,
reshaping them is not.

For the rest it is the *Implemented by* line in the header that must be read.
