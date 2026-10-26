// Offline listening and scrobbling.
export const scrobbling = {
  "offline.kicker": "In this browser",
  "offline.title": "Kept for offline",
  "offline.detail":
    "These tracks were kept in this browser for the account that last signed in, and play without a connection. Signing out deletes them from this browser. Keep more from a track's menu.",
  "offline.unsupported":
    "This browser cannot keep tracks for offline listening.",
  "offline.back": "Back to the library",
  "offline.empty": "Nothing is kept yet. A track's menu offers to keep it.",
  "offline.player": "Offline player",
  "offline.gone": "That copy is no longer in this browser.",
  "offline.forget": "Remove the offline copy of {title}",
  "offline.forgetShort": "Remove",
  "offline.forgetDetail": "Frees {size}",
  "login.offline": {
    one: "Listen to the {count} track kept in this browser",
    other: "Listen to the {count} tracks kept in this browser",
  },
  "scrobbling.title": "External scrobbling",
  "scrobbling.detail":
    "Forward what you listen to here to ListenBrainz, Maloja or Last.fm.",
  "scrobbling.none":
    "This server offers no destination. Its operator decides that.",
  "scrobbling.provider.listenbrainz": "ListenBrainz",
  "scrobbling.provider.maloja": "Maloja",
  "scrobbling.provider.lastfm": "Last.fm",
  "scrobbling.instance": "instance “{name}”",
  "scrobbling.unavailable": "Unavailable here: {reason}",
  "scrobbling.unavailable.no_application_configured":
    "no Last.fm application is declared on this server",
  "scrobbling.unavailable.browser_journey_needs_https":
    "the browser journey needs this server’s public address to be an https one — an operator can still link an account from the command line",
  "scrobbling.unavailable.unknown": "this server did not say why",
  "scrobbling.retired":
    "This server no longer offers this instance. Nothing more will leave for it — you can still withdraw the authorisation.",
  "scrobbling.notLinked": "Not linked.",
  "scrobbling.secret": "API key",
  "scrobbling.secretHint": "Taken from your account on that destination.",
  "scrobbling.link": "Link",
  "scrobbling.working": "Working…",
  "scrobbling.linkError":
    "That authorisation was refused. Check the key and try again.",
  "scrobbling.unlink": "Unlink",
  "scrobbling.unlinkError": "That link could not be withdrawn.",
  "scrobbling.connect": "Connect to Last.fm",
  "scrobbling.connectDetail":
    "Last.fm hands out no key you can paste, so it asks you there and brings you back.",
  "scrobbling.connectError": "The Last.fm journey could not be opened.",
  "scrobbling.returned": "Last.fm is linked: {name}.",
  "scrobbling.health.healthy": "Listens are leaving",
  "scrobbling.health.degraded": "The queue is not moving",
  "scrobbling.health.broken": "Broken",
  "scrobbling.healthDetail.healthy": "Nothing is stuck here.",
  "scrobbling.healthDetail.degraded":
    "The link answers, but listens are not getting through — either one is waiting on you, or the oldest has waited too long.",
  "scrobbling.healthDetail.broken":
    "The credential was refused, or this server can no longer reach that instance under the name the link was made against.",
  "scrobbling.pending": { one: "{count} waiting", other: "{count} waiting" },
  "scrobbling.retrying": {
    one: "{count} being retried",
    other: "{count} being retried",
  },
  "scrobbling.uncertainCount": {
    one: "{count} waiting on you",
    other: "{count} waiting on you",
  },
  "scrobbling.oldest": "Oldest has waited since {when}.",
  "scrobbling.lastSuccess": "Last accepted {when}.",
  "scrobbling.lastFailure": "Last refusal: {reason}.",
  "scrobbling.uncertain": "Listens only you can decide about",
  "scrobbling.uncertainDetail":
    "The connection broke after these left, so nobody knows whether the destination recorded them. Sending one again may put it twice in a listening history other people read; throwing it away leaves a gap. This server will not choose for you, and it will ask once.",
  "scrobbling.uncertainNone": "Nothing is waiting on you.",
  "scrobbling.playedAt": "Played {when}",
  "scrobbling.attempts": { one: "{count} attempt", other: "{count} attempts" },
  "scrobbling.retry": "Send it again",
  "scrobbling.discard": "Throw it away",
  "scrobbling.answered": "Answered.",
  "scrobbling.answerError":
    "That listen could not be answered. Reload the page before trying again.",
  "scrobbling.unnamedTrack": "Not in the history this account still holds",
  "scrobbling.reason.rate_limited": "the destination asked for room",
  "scrobbling.reason.retryable": "the destination did not take it",
  "scrobbling.reason.auth_broken": "the authorisation was refused",
  "scrobbling.reason.rejected": "the destination read it and refused it",
  "scrobbling.reason.destination_gone":
    "this server no longer reaches that instance under that name",
  "scrobbling.reason.credential_unreadable":
    "this server could not read the stored credential",
  "scrobbling.reason.attempts_exhausted": "it was tried too many times",
  "scrobbling.reason.unlinked": "the link was withdrawn",
  "scrobbling.reason.ambiguous": "the answer never arrived",
  "scrobbling.reason.interrupted": "the server stopped mid-flight",
} as const;
