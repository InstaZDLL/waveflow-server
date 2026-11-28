# RFC-031 — Signing in through an OpenID Connect provider

- **Status**: Proposed
- **Implemented by**: nothing yet. The *Status* field above never flips in
  this project; this line is the one that will say what is running.
- **Date**: 2026-11-28
- **Authors**: the WaveFlow project
- **Requested in**: [InstaZDLL/WaveFlow#1188](https://github.com/InstaZDLL/WaveFlow/issues/1188)
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md) (local accounts,
  the web session, its cookies and its origin check, the dedicated Subsonic
  password, the native clients' Authorization Code + PKCE),
  [RFC-017](RFC-017-sign-in-hardening.md) (the sign-in limiter),
  [RFC-019](RFC-019-exposure-hardening.md) (the setup token),
  [RFC-030](RFC-030-home-assistant-ingress.md) (the ingress listener, its
  cookies under a prefix, its origin rule)
- **Concerns**: the desktop app at
  [`InstaZDLL/WaveFlow`](https://github.com/InstaZDLL/WaveFlow) and the
  Android app, which sign in through the server's own pages and need no
  change for the first mode (decision 9), and one for the second.

## Problem

A self-hosted server usually sits beside others that already share one
identity provider — Authentik, Keycloak, Pocket ID, Authelia — so that a
person has one account, one password and one second factor for all of them,
and an administrator disables someone in one place. WaveFlow accounts are
local (RFC-002): a username, an Argon2id web password, and a separate
Subsonic password. #1188 asks for sign-in through OpenID Connect, and for
two modes:

- **alongside**: a person may sign in either through the provider or with
  the local password;
- **provider only**: no local password signs anyone in, so the provider's
  policies — its second factor, its group rules, its lockouts — cannot be
  stepped around.

The request names the settings it expects: the issuer, the client id and
secret, the scopes and the redirect address.

## What exists, and what this must keep working

Read from the code, not from the documents:

- **Signing in** is `AuthService::login` (`src/auth/authentication.rs`),
  behind `POST /api/v2/auth/login` (native) and
  `POST /api/v2/web/auth/login` (browser), both through RFC-017's limiter.
  `AuthService::issue_session_for_account` already issues a session for an
  account whose identity was proved some other way — today, a redeemed
  authorization code. That is the call this RFC ends on.
- **The browser session** (`src/api/web_session/`) is a short access token
  held by the page, a rotating refresh token in an `HttpOnly`,
  `SameSite=Strict` cookie scoped to `/api/v2/web/auth`, and a CSRF cookie.
  `validate_web_origin` compares `Origin` with `WAVEFLOW_PUBLIC_URL` when it
  is set, so a sign-in from any other address is refused with
  `origin_rejected` (#469). On the ingress listener, the cookies sit under
  the prefix and the origin is compared with `Host` (RFC-030 decisions 5
  and 6).
- **The desktop** opens `<server>/authorize?client_id&redirect_uri&code_challenge&state&device_name`
  in the system browser (`src-tauri/crates/app/src/remote/auth.rs` in
  WaveFlow). That route of the web client requires a session; without one it
  remembers the address and sends the browser to `/login`, then back. The
  consent calls `POST /api/v2/oauth/authorize`, and the desktop redeems the
  code at `POST /api/v2/oauth/token` with its verifier.
- **Accounts** have a `password_hash NOT NULL` and a role, `admin` or
  `user`. An account that must not sign in on the web already exists:
  Subsonic's `createUser` stores the hash of a random `web-disabled-…`
  value nobody knows (`create_subsonic_user`, `src/services/admin.rs`).
- **The Subsonic façade** accepts only the dedicated Subsonic password or its
  API key, never the web password, and its wire contract is frozen.
- **The outbound client** (`src/remote/outbound/`) follows no redirect and
  reads no proxy from the environment; bodies are read under a ceiling.

## Decision 1 — Authorization Code with PKCE, as a confidential client

The server is an OpenID Connect *relying party* using the **Authorization
Code flow**, with:

- **a client secret**: the server is a confidential client, which is what
  every provider named in #1188 expects of a web application;
- **PKCE with S256** on top of it, which OAuth 2.1 and RFC 9700 recommend for
  confidential clients as well, and which every one of those providers
  accepts;
- **`state`** and **`nonce`**, both 256 random bits drawn per attempt;
- **the `query` response mode**, the default. `form_post` would bring the
  answer back as a cross-site `POST`, which only a `SameSite=None` cookie
  survives, and nothing in this server is `SameSite=None`.

The implicit and hybrid flows are not offered: they put tokens in the
browser's address bar. Nor is the device flow, which has no use here (see
decision 9).

**One provider.** One issuer is configured. The link table carries the
issuer so that a second provider is not a migration, but configuring several
is not part of this RFC.

## Decision 2 — the settings

All of them in a new `src/config/oidc.rs`, each a field on `Config` with its
variable documented on it, as every setting is. OIDC is off while
`WAVEFLOW_OIDC_ISSUER` is unset, and nothing below happens.

| Variable | Default | Meaning |
|---|---|---|
| `WAVEFLOW_OIDC_ISSUER` | unset | The issuer URL. Discovery reads `<issuer>/.well-known/openid-configuration`, with one trailing `/` of the issuer removed first so that `https://idp.example/tenant/` is not asked for `//.well-known/…` (OpenID Connect Discovery §4). The issuer itself is kept as typed, for the exact comparison of decision 10. |
| `WAVEFLOW_OIDC_CLIENT_ID` | — | Required with the issuer. |
| `WAVEFLOW_OIDC_CLIENT_SECRET` | — | Required with the issuer. Never logged, never in an answer. |
| `WAVEFLOW_OIDC_SCOPES` | `openid profile` | Space- or comma-separated. `openid` is added if missing. Add `groups` for the providers that need it to send groups (decision 6). |
| `WAVEFLOW_OIDC_NAME` | `single sign-on` | The provider's name on the sign-in button: *Sign in with Authentik*. |
| `WAVEFLOW_OIDC_USERNAME_CLAIM` | `preferred_username` | The claim a new account takes its username from (decision 4). |
| `WAVEFLOW_OIDC_GROUPS_CLAIM` | `groups` | The claim that lists the person's groups. |
| `WAVEFLOW_OIDC_ALLOWED_GROUPS` | empty | When set, a person in none of these groups is refused (decision 6). |
| `WAVEFLOW_OIDC_ADMIN_GROUP` | empty | When set, the role of a linked account follows this group (decision 6). |
| `WAVEFLOW_OIDC_AUTO_CREATE` | `true` | Whether an identity linked to no account creates one (decision 4). |
| `WAVEFLOW_OIDC_ONLY` | `false` | The provider-only mode (decision 7). |
| `WAVEFLOW_SUBSONIC_ENABLED` | `true` | Whether the Subsonic façade is served at all (decision 8). Independent of OIDC. |

**The redirect address is not a setting.** It is built from
`WAVEFLOW_PUBLIC_URL`:
`<public URL>/api/v2/web/auth/oidc/callback`, the one value the operator
registers at the provider. A second setting that had to agree with the
first would be one more way to be wrong. So **OIDC requires
`WAVEFLOW_PUBLIC_URL`**: a provider redirects to a fixed address, and a
server that does not know its own cannot name one.

**The boot refuses** rather than half-enabling:

- an issuer without a client id or a secret, or the reverse;
- an issuer that is not `https`, except on a loopback host;
- an issuer without `WAVEFLOW_PUBLIC_URL`, or with a public URL that is not
  `https`, except on a loopback host. The callback brings the code, the
  `state` and the flow cookie back to that address; in clear text, whoever
  sees the request can present all three before the browser does, and the
  session the callback issues is theirs. `Secure` on the cookie would not
  help, since the code travels in the query string anyway;
- `WAVEFLOW_OIDC_ONLY=true` without an issuer;
- an admin or allowed group that is empty after trimming.

The boot does **not** reach the provider. A server whose provider is down
must still start, serve its Subsonic clients and its sessions, and say why
the sign-in button fails rather than fail to exist.

## Decision 3 — the round trip, step by step

Three routes, under `/api/v2/web/auth/oidc/`, all on the main listener:

1. **`POST /api/v2/web/auth/oidc/start`** — called by the sign-in page, with
   `Origin` checked by `validate_web_origin` as the password sign-in is. It
   takes `{ purpose, device_name, return_to }`, where `purpose` is
   `sign_in`, `link` (decision 5) or `setup` (decision 7). It answers
   `{ redirect_to }`, the provider's authorization address with
   `response_type=code`, `client_id`, `redirect_uri`, `scope`, `state`,
   `nonce`, `code_challenge` and `code_challenge_method=S256`; and it sets
   the **flow cookie** below. The page then navigates there.
2. The person signs in at the provider, which sends the browser to
   **`GET /api/v2/web/auth/oidc/callback?code&state`** (or `?error&state`).
3. The callback checks the flow, redeems the code, validates the ID token
   (decision 10), resolves the account (decision 4), issues a web session
   through `issue_session_for_account` and `web_auth_response`, sets the
   session cookies, clears the flow cookie, and answers `303` to
   `return_to`. On any failure it clears the flow cookie and answers `303`
   to `/login?oidc_error=<code>`, where `<code>` is one of a fixed
   vocabulary (decision 11).

**The flow cookie carries the whole pending attempt, sealed.** Its value is
the AEAD-sealed tuple `(kind, purpose, state, nonce, PKCE verifier,
device name, return_to, account for a link, expiry)`, under the instance
key through `SecretBox`, as stream tickets are, with a kind tag of its own so
that no other sealed value can be presented as one. It is named `wf_oidc`,
`HttpOnly`, `Max-Age=600`, `Path=/api/v2/web/auth/oidc`, `Secure` when the
public URL is `https`, and **`SameSite=Lax`** — not `Strict` like the
session's, because the callback is a cross-site navigation from the provider,
on which a `Strict` cookie is not sent. `Lax` is sent on that top-level `GET`
and on nothing cross-site that can change state.

Sealing it rather than keeping it on the server is deliberate:

- **Nothing to fill.** `start` is reachable without an account. A table or
  a map of pending attempts is something any stranger can fill, and either
  its bound refuses real people or its absence grows without one. A cookie
  costs the server nothing until it comes back.
- **The verifier never touches the disk**, and a restart forgets nothing that
  matters: an attempt in flight simply completes.
- **Bound to the browser.** The callback opens the cookie, compares its
  `state` with the query's in constant time, and refuses on any difference,
  or on a cookie that is missing, expired or does not open. That is what
  stops *login CSRF* — an attacker who sends someone to the callback with
  the attacker's own code, signing them into the attacker's account: the
  attacker cannot give the victim a cookie that names the attacker's `state`.

**Nothing reaches the provider before the cookie checks out.** The callback
verifies the flow cookie and `state` first; only then does it call the
token endpoint. A request forged at the callback without a cookie costs one
decryption.

**And what reaches it is bounded per address.** `start` is free, so anyone
can draw cookies and present each with an invented code, and each would
then cost a request to the provider — which may in turn limit this server's
address and refuse the real sign-ins. So a callback that has passed the
cookie and `state` checks takes a slot in RFC-017's limiter before the token
request, under the request's address bucket (RFC-017 decision 2, RFC-030
decision 1 on the ingress listener) and a fixed account key of its own,
`oidc`: twenty in a sliding minute, as for passwords. A slot is given back
when the round trip succeeds, and kept when the provider refuses the code,
so a person who signs in many times is never refused and a stream of
invented codes is. Past the limit the callback fails with `oidc_failed`
without reaching the provider. A flow cookie is also usable once: its
`state` is remembered as spent until the cookie's expiry, so the same
attempt cannot be replayed even within its ten minutes. Only a callback that
got past the limiter adds to that set, so it grows no faster than the
limiter allows, and it is bounded like the limiter's table besides.

**`return_to`** is a path inside the client, checked on the server, and
stricter than `safeInternalPath` (`webapp/src/api/oauth.ts`), which checks
only the first two characters. A value is refused if it contains any control
character — U+0000 to U+001F and U+007F — anywhere: a browser removes tabs
and line breaks from a URL before resolving it, so `/\t/evil.example` would
become `//evil.example`, another host. Then it is resolved against the
public URL with the same URL parser the server uses for every address, and
kept only if the result has the public URL's origin. What is sealed, and
what the callback's `Location` carries, is that **absolute** URL, never its
path alone: `/a/..//evil.example` resolves on the public origin to the path
`//evil.example`, which, extracted and sent back as a relative reference,
names another host. An absolute URL on the public origin cannot be read
that way. Anything else becomes the public URL's root. The
client's `safeInternalPath` gets the same control-character rule, in its
own pull request.
It is what carries a desktop's `/authorize?…` through the detour (decision
9). It is sealed, so the provider never sees it and the browser cannot
change it on the way back.

**The flow stays on the public address.** `start` passes
`validate_web_origin`, which, with `WAVEFLOW_PUBLIC_URL` set — and decision 2
requires it — refuses every other origin, exactly as the password sign-in has
since #469. A page opened at `http://192.168.1.10:4533` therefore says that
signing in through the provider is done from the public address, and links
to it, rather than set a cookie on an address the provider will never send
the browser back to.

## Decision 4 — which account an identity is

**An identity is the pair (issuer, `sub`), and nothing else.** OpenID Connect
guarantees that `sub` is unique and never reassigned within an issuer; no
other claim carries that promise. A username, a display name or an email
address can be edited by its owner in most providers, reused after an
account is deleted, or shared between two providers. Matching on any of them
would hand an account to whoever next carried the value.

A new table, in a dated migration:

```sql
CREATE TABLE account_oidc_identity (
    issuer TEXT NOT NULL CHECK (length(issuer) > 0),
    subject TEXT NOT NULL CHECK (length(subject) BETWEEN 1 AND 255),
    user_id TEXT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    last_sign_in_at INTEGER,
    PRIMARY KEY (issuer, subject),
    UNIQUE (user_id, issuer)
) STRICT;
```

One identity per account per issuer, one account per identity. Deleting the
account deletes its link. The issuer is stored exactly as discovery
confirmed it (decision 10), so **changing `WAVEFLOW_OIDC_ISSUER` matches no
existing link**: a different issuer is a different set of people, even when
it names the same provider moved to a new address. The operator guide says
so, and how to carry the links across (decision 5's administrative link).

At the callback, once the token is valid:

1. **A link exists** for (issuer, `sub`): that account. If it is disabled,
   the attempt is refused like a wrong password is.
2. **No link, and `WAVEFLOW_OIDC_AUTO_CREATE` is true** (the default): a
   new account is created and linked in one transaction under the writer
   gate:
   - its **username** is the `WAVEFLOW_OIDC_USERNAME_CLAIM` claim, trimmed
     and checked by the rule every username follows (`validate_name`). A
     claim that is missing, invalid or **already a WaveFlow account's name**
     refuses the attempt. It never takes the existing account over, and it
     never invents a variant — `alice-2` is not anyone's name, and would be
     what Subsonic clients have to be configured with;
   - its **role** is `user`, or `admin` when decision 6's group says so;
   - its **password** is the hash of a random value nobody knows, as
     Subsonic's `createUser` already does: the account cannot sign in with a
     password until an administrator gives it one;
   - it is a member of **no library**, as an account created on the
     administration page is, until an administrator grants some. The client
     already has a page for an account that reaches no library, and says to
     ask an administrator;
   - the creation is recorded in `audit_event` as `oidc.account_created`.
3. **No link, and auto-creation is off**: refused. Only an account an
   administrator or its owner has linked beforehand (decision 5) signs in.

**The username is fixed at creation.** A later change of
`preferred_username` at the provider renames nothing: WaveFlow's username is
what Subsonic clients sign in with, and an identity's claims are not trusted
to rename an account — only to say who is signing in.

**An existing account with the same name is never linked by its name.** Its
owner links it themselves, or an administrator does (decision 5). The
refusal does not say why (decision 11): *no account is linked to this
identity, and none could be created*. Telling the person that the name is
taken would confirm that an account of that name exists, which RFC-002's rule
says is never confirmed to someone not entitled to it.

**Email is not used.** Not to match, not to create, and `email_verified`
does not change that: a verified address proves the person received a
message, not that they are the account that once registered it.

## Decision 5 — linking an existing account, and unlinking

Two ways, and neither matches a name:

- **By its owner**, while signed in: *Settings › Account › Link to
  <provider>* calls `POST /api/v2/web/auth/oidc/start` with
  `purpose=link`, authenticated by the session's bearer token besides the
  origin check. The account's id is sealed in the flow cookie; the callback
  links the identity that comes back to it. The person has proved both
  sides — the WaveFlow session and the provider — in one gesture. It is
  refused if that identity is already linked to another account, or if this
  account already has a link to this issuer.
- **By an administrator**, who types the identity's `sub` as the provider
  shows it (Authentik's *UID*, Keycloak's *ID*, Pocket ID's *ID*, Authelia's
  *subject*): `PUT /api/v2/admin/users/{username}/oidc { subject }`, the
  administration page, and the command
  `waveflow-server account link-oidc <username> --subject <sub>`. This is the
  way for an account whose owner cannot sign in with a password — in the
  provider-only mode, for instance — and for carrying the links to a new
  issuer.

Unlinking: `DELETE /api/v2/account/oidc` by the owner, refused in the
provider-only mode where it would lock them out;
`DELETE /api/v2/admin/users/{username}/oidc` and
`waveflow-server account unlink-oidc <username>` by an administrator.

Each is an `audit_event`: `oidc.linked`, `oidc.unlinked`. An unknown
username on the administrative routes is `404`, like every other account
route.

## Decision 6 — groups: who may enter, and who is an administrator

Groups are read from `WAVEFLOW_OIDC_GROUPS_CLAIM`, an array of strings or a
single string, in the ID token or in the userinfo answer (decision 10).
They are compared exactly, case included: providers name groups as their
administrators typed them, and two groups differing only by case are two
groups.

- **`WAVEFLOW_OIDC_ALLOWED_GROUPS`**, when set, is checked at **every**
  sign-in through the provider, not only at creation: someone taken out of
  the group is refused at their next sign-in. Unset, whoever the provider
  lets through may enter — which is how most of these providers are meant to
  be used, with an access policy on the application itself.
- **`WAVEFLOW_OIDC_ADMIN_GROUP`**, when set, makes the provider the source of
  a linked account's role: at each sign-in through it, the account becomes
  `admin` when the group is listed, `user` when it is not. Two rules keep
  that from going wrong:
  - **the last enabled administrator is never demoted by it.** The sign-in
    goes on with the role unchanged and a `warn` line says that the group no
    longer lists the instance's only administrator. A misconfigured group
    must not leave a server nobody can administer;
  - **the role of a linked account is not changed by hand** while the
    setting is set: the administration page shows it as managed by the
    provider, and the API refuses the change with `409
    role_managed_by_provider`. A change the next sign-in would undo silently
    is worse than one that is refused.

  A promotion or a demotion is an `audit_event`, `oidc.role_changed`.
  Unset, the role is WaveFlow's to manage, as today, and the provider's
  groups never touch it.

**Missing groups are an empty list.** A provider that sends no groups claim —
because `groups` is not among the scopes, or a mapper is missing — makes
everyone a non-member: refused if an allowed group is set, `user` if an
admin group is. The guide gives, for each of the four providers, what makes
it send groups.

**Groups act at sign-in only.** WaveFlow keeps no token of the provider's
(decision 10), so it does not learn that someone left a group until they
sign in through it again. Their open sessions, their API tokens and their
Subsonic password keep working until then — see *What this RFC does not do*.

## Decision 7 — two modes, and how to recover from the second

**Alongside** (`WAVEFLOW_OIDC_ONLY=false`, the default once an issuer is
set): the sign-in page offers the provider's button above the password form,
and both work for every account that has what they need.

**Provider only** (`WAVEFLOW_OIDC_ONLY=true`):

- **no password signs anyone in**: `POST /api/v2/auth/login` and
  `POST /api/v2/web/auth/login` answer `403 password_sign_in_disabled`
  before the limiter, before any lookup and before any hashing — there is
  nothing to guess, so nothing to count. Administrators are not exempted:
  an exemption is the bypass the mode exists to close, and the account most
  worth stealing is the one that would have it;
- **no web password is set or changed**: the account settings do not offer
  it, the administration API refuses `web_password` with the same code, and
  creating an account through the API takes an optional `oidc_subject`
  instead of a password;
- **the sign-in page shows the provider's button alone**;
- **Subsonic's `createUser`** answers error 50, *not authorized*: it would
  create an account that the provider has never seen and that signs in with
  a password of its own. This changes the frozen contract only under a mode
  an operator chose, and the compatibility document says so;
- **the first administrator** is created through the provider: the setup
  page, given the setup token (RFC-019), calls `start` with `purpose=setup`
  and the token. The token is checked there, in constant time, as it is
  today; the identity that comes back at the callback becomes the first
  administrator, linked, and the setup gate closes. If an account was
  created in between, the callback refuses with the `409` setup already
  gives.

What stays in this mode, and why:

- **API tokens.** They are not a sign-in but a credential an account issues
  to a script, and issuing one takes a session, which takes the provider.
- **The dedicated Subsonic password**, unless decision 8 turns the façade
  off.
- **The command line.** Whoever runs `waveflow-server account …` on the
  server already holds the database and the instance key; nothing the HTTP
  surfaces refuse protects anything from them.

**Recovery is the command line, never a password left open.** When the
provider is gone — down, misconfigured, its issuer changed — the operator
sets `WAVEFLOW_OIDC_ONLY=false`, restarts, creates an administrator with
`waveflow-server account create-admin` if no account has a known password,
and signs in. Or links an account to the right identity with
`account link-oidc`. The operator guide says this in the section that
introduces the mode, not at the end of it.

**Moving an instance into the mode** is done alongside first: every person
links their account from their settings, the administration page shows who
has not, and only then does the operator set the variable. The page lists
the accounts without a link so that the switch locks out nobody by surprise.

## Decision 8 — Subsonic stays, and can be turned off

**OpenID Connect cannot serve the Subsonic API.** Its clients — Symfonium,
DSub, Feishin, Tempo — send a username with a password, a salted token or an
API key on every request; none of them can open a browser, follow a
provider's sign-in and hold its tokens. So **the dedicated Subsonic password
stays**, in both modes, as RFC-002 defines it: a password per application,
never the web password, set by the signed-in account from its settings.

It is a local secret, and in the provider-only mode it is the one that
remains. That is stated, not hidden:

- **it is not a way around the provider's sign-in.** It opens the Subsonic
  API alone, never a web session or a native one, and it is set only from a
  session that the provider opened;
- **for an operator who wants no local secret at all**,
  `WAVEFLOW_SUBSONIC_ENABLED=false` stops serving the façade: `/rest/*` is
  not mounted and answers the JSON `404` every unknown API path answers; the
  settings do not offer the Subsonic password, and its routes refuse with
  `409 subsonic_disabled`. The variable is independent of OIDC, since an
  instance used only from the web and the desktop has no use for the façade
  either way.

## Decision 9 — the desktop and the Android app

**The desktop needs no change.** It already signs in through the server's
own pages: it opens `/authorize` in the system browser, which sends a
browser with no session to `/login`. With OIDC configured, that page offers
the provider; its `start` call carries the `/authorize?…` address as
`return_to`; the callback issues the web session and returns there; the
consent and the code exchange proceed exactly as today. The desktop never
talks to the provider, never holds its tokens, and keeps the session the
server issues it, rotated as today. Both modes work, since the desktop never
sends a password itself.

**The Android app** signs in with `POST /api/v2/auth/login` today (the phone
link in WaveFlow's RFC-008). That keeps working alongside, and is refused in
the provider-only mode. To work there, it signs in as the desktop does —
`/authorize` in a Custom Tab, a redirect to a reverse-domain scheme, which
`validate_redirect_uri` already accepts, and PKCE. That change is the
Android repository's; the server needs nothing more.

**No provider token reaches a native client**, and the server's native
tokens keep their meaning: whoever holds one holds a WaveFlow session, not a
provider's.

## Decision 10 — what is checked, and what is kept

**Discovery.** Fetched on first use, then cached for an hour, through the
outbound client: no redirect followed, no proxy from the environment, the
body read under a ceiling (RFC-023). The `issuer` in the document must equal
the configured one exactly (OpenID Connect Discovery §4.3), or nothing is
used. A provider that does not list `S256` among
`code_challenge_methods_supported` is still asked with it — most omit the
field — and one that rejects it fails the sign-in with a code that says so.

**The token request.** The code, the verifier and the redirect address are
sent to the token endpoint with the client secret, by `client_secret_basic`
unless the document lists only `client_secret_post`. A request that fails
or times out fails the sign-in; nothing is retried, since a code is spent at
the provider on its first presentation, as it is here.

**The ID token**, from the token answer, on a TLS connection the server
opened itself:

- **its signature**, against the provider's `jwks_uri`, cached and fetched
  again at most once a minute when a `kid` is unknown, so that a provider's
  key rotation is followed and a stream of forged `kid`s is not a stream of
  requests;
- **its algorithm** is asymmetric — RS256, PS256, ES256 or EdDSA, the ones
  the document lists and the key's type allows. `none` and the HMAC family
  are refused: a symmetric token is keyed with the client secret, which
  every one of these providers can avoid with a signing key, and accepting
  both kinds is how algorithm confusion happens;
- **`iss`** equals the configured issuer; **`aud`** is the client id, as a
  string or as an array holding it alone. OpenID Connect Core §3.1.3.7
  requires refusing a token that names audiences the client does not trust,
  and this server trusts no other: a token issued for several clients at
  once is refused even when its **`azp`** names this one. When `azp` is
  present, it is the client id;
- **`exp`** is in the future and **`iat`** is not, with 60 seconds of
  leeway either way for clocks that disagree;
- **`nonce`** equals the one sealed in the flow cookie, in constant time;
- **`sub`** is present, non-empty, at most 255 bytes.

**Userinfo.** When the document names a `userinfo_endpoint`, it is called
with the access token, and its claims complete the ID token's, never replace
them. Its `sub` must equal the ID token's (OpenID Connect Core §5.3.2), or
the sign-in fails. Some providers — Authelia's recent versions among them —
put the username and the groups only there.

**Nothing of the provider's is kept.** Not the ID token, not the access
token, not a refresh token: once the account is resolved, they are dropped.
The link table holds the issuer and the `sub`, and the session is WaveFlow's
own, issued and rotated as every other is. Holding a provider's refresh
token would have let the server ask the provider, later, whether a person is
still allowed — at the price of a long-lived secret of someone else's in the
database, and of a server that stops working when the provider does. That
trade is left open, below.

**Implementation.** The `openidconnect` crate is the expected choice, given
the outbound client as its HTTP client so that its requests follow the rules
above. Whichever is used, each check in this decision has its test, against
a provider the integration suite runs in-process with a key it generates.

## Decision 11 — refusals, logs and the existing rules

**What the browser is told** is one code in `/login?oidc_error=`, from a
fixed list, and nothing the provider wrote:

| Code | When |
|---|---|
| `oidc_unavailable` | Discovery, the token request, the keys or userinfo failed, or timed out. |
| `oidc_cancelled` | The provider answered `access_denied` — the person declined, or a policy refused them. |
| `oidc_failed` | The flow cookie, the `state`, the ID token or userinfo did not check out, or the provider answered another error. |
| `oidc_not_allowed` | `WAVEFLOW_OIDC_ALLOWED_GROUPS` lists none of the person's groups. |
| `oidc_account_unavailable` | No account is linked and none could be created: auto-creation is off, the username claim is missing or invalid, a username is taken, or the linked account is disabled. One code for all of them, so that it never confirms that a given account exists. |
| `oidc_link_conflict` | A link was asked for, and the identity or the account is already linked. |

The provider's `error_description` is never shown: it is text from another
host, displayed on this server's page.

**What is logged.** Never a code, a `state`, a `nonce`, a verifier, a token,
the client secret or a claim's value — the `sub`, the username and the groups
included. The callback's path is traced like every other, without its query
string, as `CLAUDE.md` requires. A failure before the provider is reached —
a missing or stale flow cookie, a `state` that differs — is `debug`, since a
stranger can produce as many as they like. A failure after it — a provider
that answered something wrong, or not at all — is a `warn` line naming the
reason from the table and nothing more, which an operator needs and which
takes a real round trip to cause. Creations, links and role changes are in
`audit_event` with the account's id.

**RFC-017's limiter counts the round trips, not guesses.** Nothing here
checks a password, so the dummy hash and the verification gate do not
apply; the limiter's windows bound how many callbacks reach the provider
from one address (decision 3). The provider limits its own sign-in.

**The 404 rule holds.** The administrative routes answer `404` for an
unknown username like every other account route; the sign-in's refusals
fold every reason that would name an account into `oidc_account_unavailable`.

**The origin rule holds.** `start` goes through `validate_web_origin`; the
callback is a navigation and carries no `Origin`, and is protected instead by
the flow cookie and `state`. The session it issues is the same web session,
with the same cookies, scoped to the root of the main listener.

## Decision 12 — Home Assistant's ingress

**The provider's button is not offered in the panel.** Its callback returns
to `WAVEFLOW_PUBLIC_URL`, an address outside the panel, whose cookies the
panel never has — the same reason the panel cannot link Last.fm (RFC-030
decision 8). On the ingress listener, `start` answers
`409 oidc_unavailable_here`, and the sign-in page there says that signing in
through the provider is done from the server's own address. Registering the
ingress address at the provider instead is not possible: it carries a token
that changes with each installation of Home Assistant.

So **in the provider-only mode the panel cannot sign anyone in**, since it has
neither the password nor the provider. The panel says so. Signing a Home
Assistant user in from the `X-Remote-User-*` headers, which RFC-030
decision 7 left to a design of its own, would be the way to give the panel a
sign-in in that mode; it is not this RFC's.

## What the web client shows

- **Sign-in**: *Sign in with <name>* above the password form alongside, or
  alone in the provider-only mode; the message for each refusal code. The
  page learns which to show from `GET /api/v2/web/auth/methods`
  (`{ password: bool, oidc: { name } | null }`), unauthenticated, which says
  nothing a visitor of the page would not see on it.
- **Settings › Account**: whether the account is linked, *Link* and *Unlink*.
- **Administration › Users**: who is linked, the field for a `sub`, the role
  shown as managed by the provider under decision 6, and the list of
  accounts without a link before a switch to the provider-only mode.
- **Setup**: the provider's button beside the token in the provider-only
  mode.

Every string in the nine locales, as the client's rule requires.

## What does not change

- The password sign-in, the session, its rotation and its cookies, in the
  alongside mode and without OIDC.
- The Subsonic wire contract, except Subsonic's `createUser` in the
  provider-only mode (decision 7) and the whole façade when an operator
  turns it off (decision 8).
- The native Authorization Code + PKCE flow and its routes.
- The ingress listener's rules (RFC-030).

## What this RFC does not do

- **It does not end a session when the provider disables someone.** No token
  of the provider's is kept (decision 10), so the server hears nothing until
  the person signs in again. An administrator disables the account in
  WaveFlow too, which ends its sessions, its API tokens and its Subsonic
  password at once.
- **It does not sign out of the provider.** Signing out of WaveFlow ends
  WaveFlow's session; the provider's own session stays, so the next *Sign in
  with …* goes straight through.
- **It does not configure several providers.**
- **It does not sign anyone in from Home Assistant** (decision 12).

## The order of the work

1. **This RFC.**
2. **The server**, in one pull request: the settings and their boot checks,
   the migration, the round trip and its sealed cookie, the validation, the
   account resolution and the links, the groups, the provider-only mode and
   its refusals, `WAVEFLOW_SUBSONIC_ENABLED`, the commands, the routes in
   the OpenAPI document. Its tests run against an in-process provider
   serving discovery, keys, a token endpoint and userinfo, and include, at
   least: a forged callback without a cookie, with another attempt's cookie,
   with a differing `state`, with a cookie already spent; the twenty-first
   invented code from one address in a minute, which must not reach the
   provider; a `return_to` of `//evil.example`, `/\evil.example`,
   `/<tab>/evil.example` and `/a/..//evil.example`, none of which may send
   the browser off the public origin; an ID token with `alg: none`,
   an HMAC algorithm, an unknown `kid`, another `iss`, another `aud`, an
   `aud` naming the client and another beside an `azp` naming the client,
   an expired `exp`, another `nonce`; a userinfo `sub` that differs; a username claim
   naming an existing account, which must neither link nor sign in; the
   last administrator outside the admin group, who stays one; every password
   route refused in the provider-only mode before the limiter counts
   anything; a setup completed through the provider; the façade turned off.
   Each of these is checked to fail with its guard removed.
3. **The web client**, in the same pull request or the next: the sign-in
   button and its messages, the settings, the administration page, the setup
   page. Playwright covers the round trip against the mock server, left to
   CI.
4. **The documentation**: a section in `docs/guides/operations.md` with each
   provider's configuration — Authentik, Keycloak, Pocket ID, Authelia — the
   redirect address to register, the groups each needs, the move into the
   provider-only mode and the recovery from it; and
   `docs/compatibility/subsonic-compatibility.md` for decisions 7 and 8.
5. **The Android app**, in its repository, for the provider-only mode
   (decision 9).

## What is still open

- **Keeping the provider's refresh token**, to ask it at each WaveFlow
  refresh whether the person is still allowed, and end the session when not.
  Back-channel logout (OpenID Connect Back-Channel Logout 1.0) would do the
  same without a stored token, for the providers that send it.
- **RP-initiated logout**, through the provider's `end_session_endpoint`,
  for whoever wants *Sign out* to sign out of the provider too.
- **An HMAC-signed ID token**, for an Authentik provider left without a
  signing key: accepted with the client secret as key, or refused as now
  with a message that says to choose a key.
- **An `http` issuer or public URL on the local network**, which some home
  installations run before they have certificates. Decision 2 refuses both
  outside loopback, for the replay it describes; relaxing it would need a
  reason that outweighs it.
- **A setting for trusted extra audiences**, if a provider is found that
  puts another audience in its ID tokens by default. None of the four is
  known to; the implementation checks each against a real instance.
- **`WAVEFLOW_OIDC_CLIENT_SECRET_FILE`**, for container secrets, if other
  secrets — Last.fm's — get the same form.
- **A guard against demoting the last administrator by hand**: the
  administration page refuses only demoting oneself, so two administrators
  can still demote each other to none. Decision 6 guards its own path; the
  page's is a separate fix.
