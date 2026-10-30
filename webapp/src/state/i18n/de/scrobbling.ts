// Scrobbling.
import type { TranslationValue } from "..";
import type { scrobbling as en } from "../en/scrobbling";

export const scrobbling: Record<keyof typeof en, TranslationValue> = {
  "scrobbling.title": "Externes Scrobbling",
  "scrobbling.detail":
    "Leite weiter, was du hier hörst – an ListenBrainz, Maloja oder Last.fm.",
  "scrobbling.none":
    "Dieser Server bietet kein Ziel an. Das entscheidet sein Betreiber.",
  "scrobbling.provider.listenbrainz": "ListenBrainz",
  "scrobbling.provider.maloja": "Maloja",
  "scrobbling.provider.lastfm": "Last.fm",
  "scrobbling.instance": "Instanz „{name}“",
  "scrobbling.unavailable": "Hier nicht verfügbar: {reason}",
  "scrobbling.unavailable.no_application_configured":
    "auf diesem Server ist keine Last.fm-Anwendung hinterlegt",
  "scrobbling.unavailable.browser_journey_needs_https":
    "die Verknüpfung im Browser braucht eine öffentliche https-Adresse dieses Servers – ein Betreiber kann ein Konto aber über die Kommandozeile verknüpfen",
  "scrobbling.unavailable.unknown": "dieser Server hat keinen Grund genannt",
  "scrobbling.retired":
    "Dieser Server bietet diese Instanz nicht mehr an. Es wird nichts mehr an sie gesendet – du kannst die Autorisierung aber noch zurückziehen.",
  "scrobbling.notLinked": "Nicht verknüpft.",
  "scrobbling.secret": "API-Schlüssel",
  "scrobbling.secretHint": "Aus deinem Konto bei diesem Ziel.",
  "scrobbling.link": "Verknüpfen",
  "scrobbling.working": "Wird bearbeitet…",
  "scrobbling.linkError":
    "Diese Autorisierung wurde abgelehnt. Prüf den Schlüssel und versuch es noch einmal.",
  "scrobbling.unlink": "Trennen",
  "scrobbling.unlinkError": "Diese Verknüpfung konnte nicht getrennt werden.",
  "scrobbling.connect": "Mit Last.fm verbinden",
  "scrobbling.connectDetail":
    "Last.fm gibt keinen Schlüssel zum Einfügen heraus, daher fragt es dich dort und schickt dich zurück.",
  "scrobbling.connectError":
    "Die Verbindung zu Last.fm konnte nicht gestartet werden.",
  "scrobbling.returned": "Last.fm ist verknüpft: {name}.",
  "scrobbling.health.healthy": "Wiedergaben werden gesendet",
  "scrobbling.health.degraded": "Die Warteschlange bewegt sich nicht",
  "scrobbling.health.broken": "Gestört",
  "scrobbling.healthDetail.healthy": "Hier hängt nichts fest.",
  "scrobbling.healthDetail.degraded":
    "Die Verknüpfung antwortet, aber Wiedergaben kommen nicht durch – entweder wartet eine auf dich, oder die älteste wartet schon zu lange.",
  "scrobbling.healthDetail.broken":
    "Die Zugangsdaten wurden abgelehnt, oder dieser Server erreicht die Instanz nicht mehr unter dem Namen, für den die Verknüpfung erstellt wurde.",
  "scrobbling.pending": { one: "{count} wartet", other: "{count} warten" },
  "scrobbling.retrying": {
    one: "{count} wird erneut versucht",
    other: "{count} werden erneut versucht",
  },
  "scrobbling.uncertainCount": {
    one: "{count} wartet auf dich",
    other: "{count} warten auf dich",
  },
  "scrobbling.oldest": "Die älteste wartet seit {when}.",
  "scrobbling.lastSuccess": "Zuletzt angenommen {when}.",
  "scrobbling.lastFailure": "Letzte Ablehnung: {reason}.",
  "scrobbling.uncertain": "Wiedergaben, über die nur du entscheiden kannst",
  "scrobbling.uncertainDetail":
    "Die Verbindung brach ab, nachdem diese gesendet wurden, daher weiß niemand, ob das Ziel sie erfasst hat. Wenn du eine erneut sendest, steht sie vielleicht doppelt in einem Hörverlauf, den andere lesen; verwirfst du sie, entsteht eine Lücke. Dieser Server entscheidet nicht für dich, und er fragt nur einmal.",
  "scrobbling.uncertainNone": "Nichts wartet auf dich.",
  "scrobbling.playedAt": "Gespielt {when}",
  "scrobbling.attempts": {
    one: "{count} Versuch",
    other: "{count} Versuche",
  },
  "scrobbling.retry": "Erneut senden",
  "scrobbling.discard": "Verwerfen",
  "scrobbling.answered": "Erledigt.",
  "scrobbling.answerError":
    "Diese Wiedergabe konnte nicht beantwortet werden. Lade die Seite neu, bevor du es noch einmal versuchst.",
  "scrobbling.unnamedTrack": "Nicht mehr im Verlauf, den dieses Konto noch hat",
  "scrobbling.reason.rate_limited": "das Ziel hat um eine Pause gebeten",
  "scrobbling.reason.retryable": "das Ziel hat sie nicht angenommen",
  "scrobbling.reason.auth_broken": "die Autorisierung wurde abgelehnt",
  "scrobbling.reason.rejected": "das Ziel hat sie gelesen und abgelehnt",
  "scrobbling.reason.destination_gone":
    "dieser Server erreicht die Instanz unter diesem Namen nicht mehr",
  "scrobbling.reason.credential_unreadable":
    "dieser Server konnte die gespeicherten Zugangsdaten nicht lesen",
  "scrobbling.reason.attempts_exhausted": "sie wurde zu oft versucht",
  "scrobbling.reason.unlinked": "die Verknüpfung wurde getrennt",
  "scrobbling.reason.ambiguous": "die Antwort kam nie an",
  "scrobbling.reason.interrupted": "der Server wurde mitten im Senden gestoppt",
};
