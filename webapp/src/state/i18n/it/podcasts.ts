// Podcast subscriptions and episodes.
import type { TranslationValue } from "..";
import type { podcasts as en } from "../en/podcasts";

export const podcasts: Record<keyof typeof en, TranslationValue> = {
  "podcasts.detail":
    "I programmi che questo server segue, per tutti gli account",
  "podcasts.off":
    "I podcast sono disattivati su questo server. Ciò che era già stato salvato resta disponibile; non viene scaricato nulla di nuovo.",
  "podcasts.url": "Indirizzo del feed",
  "podcasts.shared":
    "Visibile a tutti gli account: chiunque su questo server vede il programma e può riprodurre gli episodi che scarica. Solo gli amministratori vedono l’indirizzo del feed.",
  "podcasts.subscribe": "Iscriviti",
  "podcasts.refresh": "Aggiorna tutti i feed",
  "podcasts.refreshing": "I feed vengono scaricati di nuovo.",
  "podcasts.reload": "Ricarica",
  "podcasts.empty": "Ancora nessun podcast.",
  "podcasts.emptyDetail":
    "Qui un amministratore si iscrive al feed di un programma. Il server ne scarica gli episodi, e tutti gli account di questo server possono ascoltarli.",
  "podcasts.addFirst": "Aggiungi il tuo primo podcast",
  "podcasts.channels": "Programmi",
  "podcasts.showing": "Stai vedendo gli episodi di {title}.",
  "podcasts.noEpisodes": "Questo feed non ha episodi.",
  "podcasts.episodes": { one: "{count} episodio", other: "{count} episodi" },
  "podcasts.failed": "ultimo aggiornamento non riuscito: {reason}",
  "podcasts.open": "Episodi",
  "podcasts.hide": "Nascondi",
  "podcasts.unsubscribe": "Annulla iscrizione",
  "podcasts.download": "Scarica",
  "podcasts.play": "Riproduci {title}",
  "podcasts.pause": "Metti in pausa {title}",
  "podcasts.resumeAt": "riprende da {time}",
  "podcasts.status.new": "non scaricato",
  "podcasts.status.downloading": "download in corso",
  "podcasts.status.completed": "pronto",
  "podcasts.status.error": "download non riuscito",
  "podcasts.status.deleted": "eliminato",
  "podcasts.conflict": "Questo server segue già quel feed.",
  "podcasts.refused":
    "Non è stato possibile scaricare o leggere il feed, oppure questo server non è autorizzato a raggiungere il suo indirizzo.",
  "podcasts.subscribeError": "Non è stato possibile aggiungere il feed.",
  "podcasts.mutationError": "Non ha funzionato. Riprova.",
  "podcasts.playbackError": "Impossibile riprodurre questo episodio.",
};
