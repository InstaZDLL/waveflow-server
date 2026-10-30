// Playlists and their import, the queue, shares and bookmarks.
import type { TranslationValue } from "..";
import type { playlists as en } from "../en/playlists";

export const playlists: Record<keyof typeof en, TranslationValue> = {
  "playlists.detail": { one: "{count} raccolta", other: "{count} raccolte" },
  "playlists.newName": "Nome della nuova playlist",
  "playlists.create": "Crea playlist",
  "playlists.play": "Riproduci",
  "playlists.addQueue": "Aggiungi la coda",
  "playlists.emptyOne": "Questa playlist è vuota.",
  "playlists.export": "Salva come .m3u8",
  "playlists.exportError": "Non è stato possibile salvare questa playlist.",
  "playlists.empty": "Crea una playlist per tenere insieme un gruppo di brani.",
  "playlists.createError": "Non è stato possibile creare la playlist.",
  "playlists.queueError":
    "Non è stato possibile aggiungere la coda a questa playlist.",
  "playlists.deleteError": "Non è stato possibile eliminare la playlist.",
  "playlists.public": "Visibile a tutti gli account",
  "playlists.publicError": "Non è stato possibile pubblicare questa playlist.",
  "playlists.others": "Da altri account",
  "playlists.othersDetail":
    "Le playlist pubbliche degli altri account, con i brani presenti nelle tue librerie.",
  "playlists.by": "Di {owner}",
  "playlists.moveUp": "Sposta su {title}",
  "playlists.moveDown": "Sposta giù {title}",
  "playlists.removeEntry": "Rimuovi {title} da questa playlist",
  "playlists.moveError":
    "Non è stato possibile spostare questa voce. La playlist potrebbe essere cambiata altrove; è stata ricaricata.",
  "playlists.removeEntryError": "Non è stato possibile rimuovere questa voce.",
  "import.title": "Importa playlist da un altro lettore",
  "import.detail":
    "Un file .m3u o .m3u8. Non viene creato nulla finché non hai visto cosa andrebbe perso.",
  "import.pick": "Scegli un file di playlist",
  "import.name": "Nome della playlist",
  "import.found": "{resolved} voci su {entries} sono nella tua libreria.",
  "import.encoding":
    "Il file non era in UTF-8 ed è stato letto come {encoding}. Controlla i nomi qui sotto prima di importarlo.",
  "import.missing": {
    one: "{count} voce non può essere collocata:",
    other: "{count} voci non possono essere collocate:",
  },
  "import.line": "Riga {line}",
  "import.noMatch": "nessun brano corrispondente",
  "import.ambiguous": "corrisponde a più brani",
  "import.notAPath": "non è un file di una libreria",
  "import.duplicate": "già in questa playlist",
  "import.confirm": "Importa",
  "import.cancel": "Annulla",
  "import.failed": "Non è stato possibile leggere questo file come playlist.",
  "queue.detail": {
    one: "{count} brano sincronizzato con il tuo account",
    other: "{count} brani sincronizzati con il tuo account",
  },
  "queue.playStart": "Riproduci dall’inizio",
  "queue.clear": "Svuota la coda",
  "queue.remove": "Rimuovi",
  "queue.empty":
    "Riproduci un album o una playlist per creare la tua coda sincronizzata.",
  "shares.detail": "Link pubblici per la musica che scegli",
  "shares.description": "Descrizione",
  "shares.create": "Condividi la coda attuale",
  "shares.needQueue": "Aggiungi dei brani alla coda prima di creare un link.",
  "shares.once": "Questo link viene mostrato una sola volta:",
  "shares.defaultName": "Musica condivisa",
  "shares.empty": "Non è stato creato nessun link pubblico.",
  "shares.createError": "Non è stato possibile creare la condivisione.",
  "shares.tooLong":
    "Una condivisione contiene al massimo {limit} brani, e la coda ne contiene {count}. Accorcia la coda per condividerla.",
  "shares.deleteError": "Non è stato possibile eliminare la condivisione.",
  "shares.maxVisits": "Limite di visite (facoltativo)",
  "shares.visits": { one: "{count} visita", other: "{count} visite" },
  "shares.visitsOf": "{count} visite su {max}",
  "share.kicker": "Condiviso con te",
  "share.defaultTitle": "Musica condivisa con te",
  "share.until": "Disponibile fino al {date}",
  "share.empty": "Nessuno di questi brani può più essere riprodotto.",
  "share.player": "Lettore",
  "share.unavailableTitle": "Questo link non apre nulla",
  "share.unavailableDetail":
    "Potrebbe essere scaduto, essere stato ritirato o essere già stato aperto tutte le volte consentite.",
  "bookmarks.title": "Posizioni salvate",
  "bookmarks.detail":
    "Una per brano. Salvando di nuovo, la posizione si sposta.",
  "bookmarks.save": "Salva questa posizione",
  "bookmarks.forget": "Dimentica",
};
