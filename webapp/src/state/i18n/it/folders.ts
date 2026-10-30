// Folders, filing and uploads.
import type { TranslationValue } from "..";
import type { folders as en } from "../en/folders";

export const folders: Record<keyof typeof en, TranslationValue> = {
  "folders.count": {
    one: "{count} cartella creata in questa libreria",
    other: "{count} cartelle create in questa libreria",
  },
  "folders.where": "Percorso della cartella",
  "folders.subfolders": "Cartelle",
  "folders.noLibrary": "Nessuna libreria è disponibile per questo account.",
  "folders.none":
    "In questa libreria non è stata ancora creata nessuna cartella.",
  "folders.system": "Gestita da WaveFlow",
  "folders.systemNote":
    "«Da sistemare» raccoglie tutti i brani che non sono in nessuna cartella. È WaveFlow a gestirla: non rientra nel conteggio delle cartelle qui sopra e non si può rinominare, spostare o eliminare.",
  "filed.unfiled": "Da sistemare",
  "filed.automatic": "Automatica",
  "filed.topLevel": "Livello principale",
  "filed.here": "Qui ora",
  "filed.destinations": "Cartelle tra cui scegliere",
  "filed.loadError": "Non è stato possibile caricare le cartelle.",
  "filed.moveFailed": "Lo spostamento non è riuscito. Riprova.",
  "filed.moveTrack": "Sposta «{title}» in una cartella",
  "filed.moveFolder": "Sposta «{name}»",
  "filed.note":
    "Cartelle che esistono solo in WaveFlow. Sistemare qui un brano non sposta nessun file.",
  "filed.newFolder": "Nuova cartella",
  "filed.name": "Nome della cartella",
  "filed.renameLabel": "Nuovo nome",
  "filed.create": "Crea",
  "filed.save": "Salva",
  "filed.cancel": "Annulla",
  "filed.rename": "Rinomina",
  "filed.move": "Sposta…",
  "filed.delete": "Elimina",
  "filed.confirmDelete":
    "Eliminare «{name}» e le sue sottocartelle? I loro brani tornano in «Da sistemare»; nessun file viene toccato.",
  "filed.created": "«{name}» creata.",
  "filed.renamed": "Rinominata in «{name}».",
  "filed.deleted": {
    one: "Cartella eliminata: {count} brano è tornato in «Da sistemare».",
    other: "Cartella eliminata: {count} brani sono tornati in «Da sistemare».",
  },
  "filed.filed": {
    one: "{count} brano sistemato in {folder}.",
    other: "{count} brani sistemati in {folder}.",
  },
  "filed.folderMoved": "«{name}» spostata.",
  "filed.nameTaken": "Qui c’è già una cartella con questo nome.",
  "filed.refused":
    "Quella cartella non può andare lì: controlla il nome, oppure l’albero è troppo profondo o pieno.",
  "filed.failed": "Non ha funzionato. Riprova.",
  "filed.missing": "Questa cartella non esiste più.",
  "filed.empty": "Qui non è stato ancora sistemato nulla.",
  "filed.allFiled": "Ogni brano è stato sistemato.",
  "filed.uploadHere": "Carica qui",
  "filed.unfiledNote":
    "Tutti i brani che non sono ancora in nessuna cartella. È WaveFlow a gestire questo elenco: un brano ne esce appena viene sistemato, e l’elenco non si può rinominare né eliminare. Per sistemare un brano, apri il suo menu e scegli «Sposta in una cartella…».",
  "filed.unfiledNoteReader":
    "Tutti i brani che non sono ancora in nessuna cartella. È WaveFlow a gestire questo elenco: un brano ne esce appena viene sistemato.",
  "upload.into": "I file ricevuti finiscono nella cartella {folder}.",
  "upload.doneUnfiled": "Aggiunto alla libreria, in «Da sistemare»",
  "upload.folderMissing":
    "La cartella per cui è stato aperto questo caricamento non si trova: i file ricevuti finiranno in «Da sistemare».",
  "upload.folderUnchecked":
    "Non è stato possibile verificare la cartella per cui è stato aperto questo caricamento, quindi per ora non viene accettato nessun file.",
  "upload.checkAgain": "Verifica di nuovo",
  "upload.title": "Carica in {library}",
  "upload.detail":
    "Ogni file riceve prima un’impronta nel tuo browser, così un file che la libreria contiene già non viene mai inviato di nuovo, e un trasferimento interrotto riprende da dove si era fermato.",
  "upload.pick": "Scegli file audio",
  "upload.formats": "Formati che la libreria può indicizzare: {extensions}",
  "upload.noLibrary": "Scegli prima una libreria.",
  "upload.notAllowed":
    "Solo un proprietario o un gestore di questa libreria può aggiungervi file.",
  "upload.closed":
    "Questa libreria non accetta file. Lo decide chi gestisce il server.",
  "upload.waiting": "In attesa",
  "upload.hashing": "Calcolo dell’impronta…",
  "upload.negotiating": "Richiesta al server…",
  "upload.uploading": "Invio…",
  "upload.committing": "Aggiunta alla libreria…",
  "upload.done": "Aggiunto alla libreria",
  "upload.present": "Già nella libreria",
  "upload.refused": "Rifiutato: {reason}",
  "upload.failed": "Non riuscito: {reason}",
  "upload.progress": "Avanzamento: {name}",
  "upload.reason.unsupportedFormat":
    "non è un formato audio che la libreria può indicizzare",
  "upload.reason.tooLarge": "è più grande di quanto accetta il server",
  "upload.reason.quotaExceeded": "la libreria non ha più spazio",
  "upload.reason.libraryClosed": "la libreria non accetta file",
  "upload.reason.tooManySessions":
    "troppi trasferimenti già aperti: riprova tra poco",
  "upload.reason.refusedFile":
    "ciò che è arrivato non corrisponde al file, oppure non è audio che la libreria può leggere",
  "upload.reason.unavailable":
    "il server non ha potuto accettarlo in questo momento: riprova",
  "upload.reason.network": "la connessione si è interrotta",
  "upload.reason.notAllowed": "non puoi più aggiungere file a questa libreria",
  "upload.reason.error": "qualcosa è andato storto",
  "upload.reason.hashing":
    "questo browser non è riuscito a calcolare l’impronta del file",
};
