# RFC-030 — The web client inside Home Assistant (ingress)

- **Status**: Proposed
- **Implemented by**: nothing yet. The *Status* field above never flips in
  this project; this line is the one that says what is running.
- **Date**: 2026-11-27
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md) (the web session,
  its cookies and its origin check), [RFC-019](RFC-019-exposure-hardening.md)
  (the security headers, the trusted proxies),
  [RFC-020](RFC-020-playlists-and-shares.md) (the share page and its address),
  [RFC-022](RFC-022-playback-and-web-client.md) (the installable client, then
  offline listening and its service worker)
- **Concerns**: the Home Assistant app at
  [`InstaZDLL/waveflow-home-assistant`](https://github.com/InstaZDLL/waveflow-home-assistant),
  a repository of its own, whose `config.yaml` turns ingress on once the
  server supports it.

## Problem

The Home Assistant app runs the server's own image under the Supervisor and
publishes port 4533. Its `webui` opens the web client at
`http://<host>:4533`, in a new tab, on an address of its own. What Home
Assistant calls *ingress* is the other way: the app's page shown inside Home
Assistant's interface, in its sidebar, reached through Home Assistant's own
address and sign-in, with no port to open or to remember.

The app was published without it because the web client assumes it is served
at the root of its origin. Ingress serves it under a path that Home Assistant
chooses per installation, `/api/hassio_ingress/<token>/`, and every
assumption of a root breaks there:

- the shell asks for `/assets/index-….js`, `/manifest.webmanifest` and
  `/favicon.svg`; the stylesheet asks for its fonts at `/assets/….woff2`
  (measured in the build output);
- the client names `/api/v2/…` in some two hundred places, most through one
  function and a dozen directly (`fetch` for the artwork, the scan events,
  uploads, the setup, sign-in and refresh requests, the probe, the share);
- the server answers tickets as root paths (`/api/v2/stream/…`,
  `/api/v2/clip-stream/…`, `/api/v2/canvas-stream/…`) that the client hands
  to `<audio>` and `<video>` as they are;
- the router knows its routes from `/`, and sends a signed-out visitor to
  `/login` through `location.assign`;
- the refresh cookie is scoped to `Path=/api/v2/web/auth`, which the browser
  never sees under the prefix;
- the service worker registers `/sw.js` with the scope `/`, which under
  ingress is Home Assistant's root, not ours.

Each of these reaches Home Assistant instead of the server, which answers 404
or its own page.

## What Home Assistant does

Read from the source, not from a description of it:
[`homeassistant/components/hassio/ingress.py`](https://github.com/home-assistant/core/blob/dev/homeassistant/components/hassio/ingress.py)
in Core and
[`supervisor/api/ingress.py`](https://github.com/home-assistant/supervisor/blob/main/supervisor/api/ingress.py)
in the Supervisor, and the
[app presentation guide](https://developers.home-assistant.io/docs/apps/presentation#ingress).

- **The path is stripped.** A request for
  `/api/hassio_ingress/<token>/api/v2/albums` reaches the app as
  `GET /api/v2/albums` on its `ingress_port`, query string kept. The server
  sees the paths it already serves.
- **The prefix is told.** Core adds `X-Ingress-Path:
  /api/hassio_ingress/<token>` (and the same value as `X-Forwarded-Prefix`),
  `X-Forwarded-Host` and `X-Forwarded-Proto` from the browser's request, and
  `X-Hass-Source: core.ingress`.
- **The rest is passed through.** `Host`, `Origin`, `Cookie`, `Range` and
  `Authorization` are copied as the browser sent them; only the length,
  encoding and websocket negotiation headers are dropped. So `Host` and
  `Origin` both name Home Assistant's address, and agree.
- **Home Assistant signs the user in.** The Supervisor adds
  `X-Remote-User-Id`, `X-Remote-User-Name`, `X-Remote-User-Display-Name` and
  `X-Remote-User-Groups` for the Home Assistant user the ingress session
  belongs to, after dropping any the browser sent.
- **One peer.** Every ingress request comes from the Supervisor, at
  `172.30.32.2`, and the guide asks an app to refuse every other address on
  its ingress port.
- **Bodies are read whole unless the app says otherwise.** Without
  `ingress_stream: true`, a `POST` is buffered in memory before it reaches
  the app; responses of 4 MB and more are streamed back either way.
- **The page is framed.** The panel is an `<iframe>` in Home Assistant's own
  page, on the same origin.

## Decision 1 — a listener of its own, off by default

Ingress is served on **a second listener**, `WAVEFLOW_INGRESS_BIND` (unset by
default, and nothing below happens while it is), which accepts connections
only from the addresses in `WAVEFLOW_INGRESS_PEERS` (default `172.30.32.2`)
and closes any other at once.

A request is an *ingress request* when it arrived on that listener, and only
then. **The main listener never reads `X-Ingress-Path`**: the port the
Subsonic apps and the LAN use stays as it is, and a header anyone can send
there changes nothing. Sharing the main port and deciding by the peer address
was the alternative, and was not taken: it would put a header-driven mode on
the port that faces the network, and make every rule below depend on a check
that one forgotten call site could skip.

The app sets `WAVEFLOW_INGRESS_BIND=0.0.0.0:8099` through its `environment`,
and `ingress_port: 8099` in its `config.yaml`. 8099 is the port the guide
expects, so the app keeps 4533 published for the Subsonic apps and the
browser tab, unchanged.

## Decision 2 — the prefix comes from the header, checked

On the ingress listener, `X-Ingress-Path` must match
`^/api/hassio_ingress/[A-Za-z0-9_-]{1,128}$`. A request without it, or with
anything else, is answered `400` rather than served at the root: the
listener has no other client, and a page served at the wrong root is the
failure this RFC exists to remove. The value is carried as a request
extension, `IngressPath`, and read only from there.

## Decision 3 — the build is relative, the shell is rewritten

- **Vite builds with `base: "./"`.** The stylesheet then reaches its fonts as
  `url(./x.woff2)`, relative to itself, and code-split chunks are found from
  `import.meta.url`. Both work under any prefix with nothing to rewrite.
- **The server rewrites the shell**, the one file whose relative references
  would resolve against the page's own address — wrong on any client route
  deeper than one segment, `/albums/<id>` included, ingress or not. Every
  `src` and `href` in `index.html` that the build left relative (`./…`) or
  rooted (`/…`) is made to start at the root of the request: `/` on the main
  listener, `<prefix>/` on the ingress one. The
  rewrite is done once per prefix and cached, as the Content-Security-Policy
  already is, and the policy is computed on the rewritten text.
- **The shell carries the prefix to the client** in
  `<meta name="waveflow-base" content="<prefix>">`, empty on the main
  listener. Not a `<base href>`: the policy sets `base-uri 'none'`, and
  relaxing it to make every relative URL in the client mean something else
  would be the larger change.

## Decision 4 — one way to name a server path in the client

The client reads the prefix once, at boot, and every server path goes through
one function, `serverUrl(path)`, which prepends it to a path that starts with
`/`:

- `request`, and with it every `call` and `collect`, applies it, so the two
  hundred call sites keep their `/api/v2/…` literals unchanged;
- the direct `fetch`es (artwork, canvas upload, scan events, uploads, setup,
  sign-in, refresh, the probe, the public share) apply it themselves;
- the URLs the server answers for tickets, which are root paths, are passed
  through it before reaching an element;
- the router is created with `basepath` set to the prefix, and the
  `location.assign` calls that name a client route go through the router or
  through `serverUrl`.

Two checks keep a call added later from slipping past the prefix, each
covering what the other cannot:

- **A source check**, run with the unit tests, flags a string or template
  literal starting with `/` — `/api/`, `/share/`, `/health`, `/sw.js` alike —
  passed directly to `fetch`, `new URL`, `new EventSource`,
  `location.assign` or `location.replace`, or set as a `src` or `href`,
  unless it goes through `serverUrl`. It reads literals only: a path built in a
  variable, by concatenation or by a helper, is out of its reach, and it says
  so in its own comment rather than passing for complete.
- **The prefixed end-to-end suite** (see *The order of the work*) catches what
  the source check cannot: its mock serves the client only under the prefix
  and fails the test on any request that reaches the root, whatever built
  its address. Its reach is the paths it runs: the mock sees only the
  requests those tests make, so a call no test exercises is covered by
  neither check. Hence the rule in *The order of the work*.

The server's own answers do not change: a ticket stays `/api/v2/stream/…`.
The client is the one place that knows where it was loaded from, and the
desktop, which reads the same answers, is never under a prefix.

## Decision 5 — cookies follow the prefix

On an ingress request, the cookies the server sets are scoped under the
prefix: the refresh cookie to `<prefix>/api/v2/web/auth`, the CSRF cookie to
`<prefix>/`, and their clearing likewise. On the main listener they are
unchanged. A cookie scoped to `/` under ingress would be sent with every
request to Home Assistant itself, and the refresh cookie, scoped as it is
today, would never be sent back at all.

## Decision 6 — the origin check reads Home Assistant's address

`validate_web_origin` compares `Origin` with `WAVEFLOW_PUBLIC_URL` when it is
set, and with `Host` otherwise. Under ingress, both `Origin` and `Host` name
Home Assistant, so the comparison with `Host` holds — but an operator who set
`WAVEFLOW_PUBLIC_URL` for the server's own address would see every sign-in in
the panel refused. On an ingress request the check therefore ignores
`WAVEFLOW_PUBLIC_URL` and compares `Origin` with `Host`, which the Supervisor
passes through from the browser. `X-Forwarded-Host` is not used: on a
connection only the Supervisor can open it would say the same thing, and one
source is easier to keep right than two.

## Decision 7 — signing in is WaveFlow's, as everywhere

The panel shows WaveFlow's own sign-in page, and the session is a WaveFlow
session like any other. The `X-Remote-User-*` headers are not read: mapping a
Home Assistant user onto a WaveFlow account — matching, creating one, deciding
who is an administrator — is a design of its own, and an RFC of its own if it
is wanted. The listener of decision 1 is what would make it safe to read them
later.

## Decision 8 — what does not work in the panel, and says so

- **Offline listening.** No service worker is registered under a prefix: its
  scope would have to be the prefix, and the token in it changes with each
  installation of Home Assistant, which strands what a worker cached. The
  menu entry *Keep for offline* is not offered there, as it is not in a
  browser without Cache Storage.
- **Installing the client.** The manifest is not linked from the shell under a
  prefix; Home Assistant's own app is what a phone installs.
- **Linking Last.fm.** The callback comes back to `WAVEFLOW_PUBLIC_URL`, an
  address outside the panel whose cookies the panel never had. The scrobbling
  page says that this one link is made from the server's own address; linking
  ListenBrainz and Maloja, which take a token, works in the panel.
- **Share links** are built from `WAVEFLOW_PUBLIC_URL` as they are today, and
  never from the ingress address, which only signed-in Home Assistant users
  can open. That setting is optional, and without it the server answers a
  share's address as a bare path, `/share/<token>`: in the panel that names
  a page of Home Assistant's, and passed through `serverUrl` it would name
  one only Home Assistant's users can open. Neither is a link to send. So on
  an ingress request with no public URL, `POST /api/v2/shares` is refused
  with `409 share_address_missing` before anything is created, and the shares
  listed carry no `url`. The shares page, answered that code, takes its form
  away and says that a link needs the server's external address,
  `WAVEFLOW_PUBLIC_URL`. On the main listener nothing changes: a bare path
  there is the server's own address. The prefixed suite covers the refusal,
  the list without addresses, and the page's message.

## Decision 9 — framing

The shell is already served with `frame-ancestors 'self'` and
`X-Frame-Options: SAMEORIGIN`, and under ingress
Home Assistant's page and ours share an origin, so the panel's `<iframe>` is
allowed as it is. Nothing changes; a test holds it, since tightening the
header to `'none'` would break the panel without failing anything else.

## Decision 10 — the app's side

In `waveflow-home-assistant`, once a server release carries this:

```yaml
ingress: true
ingress_port: 8099
ingress_stream: true # uploads are not buffered in the Supervisor's memory
panel_icon: mdi:music
panel_title: WaveFlow
environment:
  WAVEFLOW_INGRESS_BIND: "0.0.0.0:8099"
```

`webui` and the published 4533 stay: the Subsonic apps need the port, and the
browser tab remains for whoever prefers it.

## What this RFC does not do

- It does not sign anyone in from Home Assistant (decision 7).
- It does not serve the Subsonic API or the native API to other clients
  through ingress: those keep port 4533.
- It does not make the client work under an arbitrary reverse-proxy prefix.
  The same machinery would, given a way to name the prefix on the main
  listener, and that is a separate decision about trusting a header there.

## The order of the work

1. **Server**: the ingress listener and its peer check, the checked prefix as
   an extension, the rewritten shell and its policy, the cookies, the origin
   check. Integration tests drive a router with the extension set.
2. **Web client**: `base: "./"`, `serverUrl`, the router's `basepath`, the
   direct calls, the tickets, no service worker and no manifest under a
   prefix, the source check. Playwright runs one suite under a prefix behind
   a mock that strips it, as Home Assistant does, and that fails on any
   request reaching the root instead. **A call that builds its path at run
   time** — in a variable, by concatenation, or through a helper other than
   `serverUrl` — comes with a test in that suite that goes through it, in the
   pull request that adds the call: the source check cannot see such a path,
   and the suite sees only what it runs.
3. **The app**: the `config.yaml` above, in its repository, at the first
   server release that carries both, tried on a real Home Assistant.

1 and 2 may land as one pull request: neither does anything without the
other, and the server half alone changes nothing a test outside it can see.

## What is still open

- Whether to answer a request on the ingress listener from a peer that is
  not trusted with a `403` instead of closing the connection. Closing is what
  the guide asks for, and tells a scanner nothing.
- Whether *Keep for offline* could be offered in the panel later, with a
  worker scoped to the prefix and its cache cleared when the prefix changes.
