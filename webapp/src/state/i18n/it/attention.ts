// The attention lists and artist splits.
import type { TranslationValue } from "..";
import type { attention as en } from "../en/attention";

export const attention: Record<keyof typeof en, TranslationValue> = {
  "attention.lists": "Elenchi",
  "attention.total": {
    one: "{count} cosa da controllare",
    other: "{count} cose da controllare",
  },
  "attention.nothing": "Non c’è nulla da controllare.",
  "attention.notYours":
    "Solo i proprietari e i gestori di questa libreria vedono cosa c’è da controllare.",
  "attention.empty": "Niente in questo elenco.",
  "attention.tracksNote":
    "Correggi un brano dal suo menu: una volta corretto, esce dall’elenco.",
  "attention.creditsNote":
    "Ognuno di questi crediti nomina più artisti in un’unica stringa. Dividilo negli artisti che nomina, oppure tienilo unito se si tratta di un solo nome. Ogni scansione successiva applica la decisione.",
  "attention.pending": "Da decidere",
  "attention.decided": "Decisi",
  "attention.split": "Dividi",
  "attention.confirmSplit": "Conferma: dividi in {count}",
  "attention.keep": "Tieni unito",
  "attention.splitDone": "{name} è stato diviso.",
  "attention.keepDone": "{name} resta unito.",
  "attention.undone": "La decisione su {name} è stata annullata.",
  "attention.alreadyDecided":
    "Nel frattempo qualcuno ha deciso su questo credito. L’elenco è stato ricaricato.",
  "attention.failed": "Non ha funzionato. Riprova.",
  "attention.noCredits": "Nessun credito è in attesa di una decisione.",
  "attention.noDecisions": "Non è stato ancora deciso nulla.",
  "attention.fragments": "In cosa verrebbe diviso {name}",
  "attention.knownFragment": "Già presente come artista in questa libreria",
  "attention.decidedSplit": "Diviso in {parts}",
  "attention.decidedKeep": "Tenuto unito",
  "attention.decidedBy": "Da {who}, il {when}",
  "attention.undo": "Annulla",
  "attention.duplicateGroup": "Probabile duplicato {group}",
  "attention.split_credits": "Artisti da dividere",
  "attention.missing_artist": "Artista mancante",
  "attention.missing_album": "Album mancante",
  "attention.missing_year": "Anno mancante",
  "attention.missing_track_number": "Numero di traccia mancante",
  "attention.missing_cover": "Copertina mancante",
  "attention.album_year_conflict": "Album con più anni",
  "attention.compilation_no_album_artist":
    "Compilation senza artista dell’album",
  "attention.duplicate_track_number": "Numeri di traccia duplicati",
  "attention.track_number_gap": "Buchi nei numeri di traccia",
  "attention.probable_duplicate": "Probabili duplicati",
  "attention.probable_fake_lossless": "Probabili falsi lossless",
  "artistSplit.offer": "Dividi in {count} artisti",
  "artistSplit.confirm": "Conferma: dividi in {count}",
  "artistSplit.done": "Diviso in {parts}.",
  "artistSplit.failed":
    "Non è stato possibile dividere questo credito. Riprova.",
};
