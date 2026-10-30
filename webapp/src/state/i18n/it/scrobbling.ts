// Scrobbling.
import type { TranslationValue } from "..";
import type { scrobbling as en } from "../en/scrobbling";

export const scrobbling: Record<keyof typeof en, TranslationValue> = {
  "scrobbling.title": "Scrobbling esterno",
  "scrobbling.detail":
    "Inoltra ciò che ascolti qui a ListenBrainz, Maloja o Last.fm.",
  "scrobbling.none":
    "Questo server non offre nessuna destinazione. Lo decide chi lo gestisce.",
  "scrobbling.provider.listenbrainz": "ListenBrainz",
  "scrobbling.provider.maloja": "Maloja",
  "scrobbling.provider.lastfm": "Last.fm",
  "scrobbling.instance": "istanza «{name}»",
  "scrobbling.unavailable": "Non disponibile qui: {reason}",
  "scrobbling.unavailable.no_application_configured":
    "su questo server non è dichiarata nessuna applicazione Last.fm",
  "scrobbling.unavailable.browser_journey_needs_https":
    "il collegamento dal browser richiede che l’indirizzo pubblico di questo server sia in https; chi gestisce il server può comunque collegare un account dalla riga di comando",
  "scrobbling.unavailable.unknown": "questo server non ha detto perché",
  "scrobbling.retired":
    "Questo server non offre più questa istanza. Non le verrà inviato più nulla; puoi comunque revocare l’autorizzazione.",
  "scrobbling.notLinked": "Non collegato.",
  "scrobbling.secret": "Chiave API",
  "scrobbling.secretHint": "Presa dal tuo account su quella destinazione.",
  "scrobbling.link": "Collega",
  "scrobbling.working": "In corso…",
  "scrobbling.linkError":
    "L’autorizzazione è stata rifiutata. Controlla la chiave e riprova.",
  "scrobbling.unlink": "Scollega",
  "scrobbling.unlinkError": "Non è stato possibile rimuovere il collegamento.",
  "scrobbling.connect": "Connetti a Last.fm",
  "scrobbling.connectDetail":
    "Last.fm non fornisce una chiave da incollare, quindi te lo chiede sul suo sito e poi ti riporta qui.",
  "scrobbling.connectError":
    "Non è stato possibile aprire il collegamento con Last.fm.",
  "scrobbling.returned": "Last.fm è collegato: {name}.",
  "scrobbling.health.healthy": "Gli ascolti partono",
  "scrobbling.health.degraded": "La coda è ferma",
  "scrobbling.health.broken": "Non funziona",
  "scrobbling.healthDetail.healthy": "Qui non c’è nulla di bloccato.",
  "scrobbling.healthDetail.degraded":
    "Il collegamento risponde, ma gli ascolti non passano: o uno aspetta una tua decisione, o il più vecchio aspetta da troppo tempo.",
  "scrobbling.healthDetail.broken":
    "La credenziale è stata rifiutata, oppure questo server non riesce più a raggiungere quell’istanza con il nome con cui è stato creato il collegamento.",
  "scrobbling.pending": {
    one: "{count} in attesa",
    other: "{count} in attesa",
  },
  "scrobbling.retrying": {
    one: "{count} in nuovo tentativo",
    other: "{count} in nuovo tentativo",
  },
  "scrobbling.uncertainCount": {
    one: "{count} attende una tua decisione",
    other: "{count} attendono una tua decisione",
  },
  "scrobbling.oldest": "Il più vecchio aspetta dal {when}.",
  "scrobbling.lastSuccess": "Ultimo accettato il {when}.",
  "scrobbling.lastFailure": "Ultimo rifiuto: {reason}.",
  "scrobbling.uncertain": "Ascolti su cui puoi decidere solo tu",
  "scrobbling.uncertainDetail":
    "La connessione si è interrotta dopo l’invio di questi ascolti, quindi nessuno sa se la destinazione li ha registrati. Inviarne uno di nuovo potrebbe farlo comparire due volte in una cronologia di ascolto che altre persone leggono; scartarlo lascia un buco. Questo server non sceglierà per te, e te lo chiederà una sola volta.",
  "scrobbling.uncertainNone": "Niente aspetta una tua decisione.",
  "scrobbling.playedAt": "Ascoltato il {when}",
  "scrobbling.attempts": {
    one: "{count} tentativo",
    other: "{count} tentativi",
  },
  "scrobbling.retry": "Invialo di nuovo",
  "scrobbling.discard": "Scartalo",
  "scrobbling.answered": "Fatto.",
  "scrobbling.answerError":
    "Non è stato possibile registrare la tua decisione su questo ascolto. Ricarica la pagina prima di riprovare.",
  "scrobbling.unnamedTrack":
    "Non presente nella cronologia che questo account conserva ancora",
  "scrobbling.reason.rate_limited": "la destinazione ha chiesto di rallentare",
  "scrobbling.reason.retryable": "la destinazione non l’ha accettato",
  "scrobbling.reason.auth_broken": "l’autorizzazione è stata rifiutata",
  "scrobbling.reason.rejected": "la destinazione l’ha letto e rifiutato",
  "scrobbling.reason.destination_gone":
    "questo server non raggiunge più quell’istanza con quel nome",
  "scrobbling.reason.credential_unreadable":
    "questo server non è riuscito a leggere la credenziale salvata",
  "scrobbling.reason.attempts_exhausted": "è stato riprovato troppe volte",
  "scrobbling.reason.unlinked": "il collegamento è stato rimosso",
  "scrobbling.reason.ambiguous": "la risposta non è mai arrivata",
  "scrobbling.reason.interrupted": "il server si è fermato a metà invio",
};
