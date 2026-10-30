// The attention lists and artist splits.
import type { TranslationValue } from "..";
import type { attention as en } from "../en/attention";

export const attention: Record<keyof typeof en, TranslationValue> = {
  "attention.lists": "Listen",
  "attention.total": {
    one: "{count} Punkt zum Ansehen",
    other: "{count} Punkte zum Ansehen",
  },
  "attention.nothing": "Nichts braucht deine Aufmerksamkeit.",
  "attention.notYours":
    "Nur Eigentümer und Verwalter dieser Bibliothek sehen, was darin Aufmerksamkeit braucht.",
  "attention.empty": "Nichts in dieser Liste.",
  "attention.tracksNote":
    "Korrigiere einen Titel über sein Menü: Sobald er korrigiert ist, verschwindet er aus der Liste.",
  "attention.creditsNote":
    "Jede dieser Künstlerangaben nennt mehrere Künstler in einer Zeichenkette. Teile sie in die genannten Künstler auf oder behalte sie als eine, wenn es ein einzelner Name ist. Jeder spätere Scan wendet die Entscheidung an.",
  "attention.pending": "Zu entscheiden",
  "attention.decided": "Entschieden",
  "attention.split": "Aufteilen",
  "attention.confirmSplit": "Bestätigen: in {count} aufteilen",
  "attention.keep": "Als eine behalten",
  "attention.splitDone": "{name} ist aufgeteilt.",
  "attention.keepDone": "{name} bleibt eine Angabe.",
  "attention.undone": "Die Entscheidung zu {name} ist rückgängig gemacht.",
  "attention.alreadyDecided":
    "Über diese Künstlerangabe wurde inzwischen entschieden. Die Liste wurde neu geladen.",
  "attention.failed": "Das hat nicht geklappt. Versuch es noch einmal.",
  "attention.noCredits": "Keine Künstlerangabe wartet auf eine Entscheidung.",
  "attention.noDecisions": "Noch ist nichts entschieden.",
  "attention.fragments": "Worin {name} aufgeteilt würde",
  "attention.knownFragment": "Bereits ein Künstler in dieser Bibliothek",
  "attention.decidedSplit": "Aufgeteilt in {parts}",
  "attention.decidedKeep": "Als eine behalten",
  "attention.decidedBy": "Von {who}, {when}",
  "attention.undo": "Rückgängig",
  "attention.duplicateGroup": "Wahrscheinliches Duplikat {group}",
  "attention.split_credits": "Aufzuteilende Künstler",
  "attention.missing_artist": "Künstler fehlt",
  "attention.missing_album": "Album fehlt",
  "attention.missing_year": "Jahr fehlt",
  "attention.missing_track_number": "Titelnummer fehlt",
  "attention.missing_cover": "Cover fehlt",
  "attention.album_year_conflict": "Alben mit mehreren Jahren",
  "attention.compilation_no_album_artist": "Compilations ohne Albuminterpret",
  "attention.duplicate_track_number": "Doppelte Titelnummern",
  "attention.track_number_gap": "Lücken in den Titelnummern",
  "attention.probable_duplicate": "Wahrscheinliche Duplikate",
  "attention.probable_fake_lossless": "Wahrscheinlich unecht verlustfrei",
  "artistSplit.offer": "In {count} Künstler aufteilen",
  "artistSplit.confirm": "Bestätigen: in {count} aufteilen",
  "artistSplit.done": "Aufgeteilt in {parts}.",
  "artistSplit.failed":
    "Diese Künstlerangabe konnte nicht aufgeteilt werden. Versuch es noch einmal.",
};
