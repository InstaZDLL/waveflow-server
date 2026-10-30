// Folders, filing and uploads.
import type { TranslationValue } from "..";
import type { folders as en } from "../en/folders";

export const folders: Record<keyof typeof en, TranslationValue> = {
  "folders.count": {
    one: "{count} Ordner in dieser Bibliothek angelegt",
    other: "{count} Ordner in dieser Bibliothek angelegt",
  },
  "folders.where": "Ordnerpfad",
  "folders.subfolders": "Ordner",
  "folders.noLibrary": "Für dieses Konto ist keine Bibliothek verfügbar.",
  "folders.none": "In dieser Bibliothek wurde noch kein Ordner angelegt.",
  "folders.system": "Von WaveFlow verwaltet",
  "folders.systemNote":
    "„Unsortiert“ sammelt alle Titel, die in keinem Ordner liegen. WaveFlow verwaltet ihn selbst: Er zählt nicht zu den Ordnern oben und kann nicht umbenannt, verschoben oder gelöscht werden.",
  "filed.unfiled": "Unsortiert",
  "filed.automatic": "Automatisch",
  "filed.topLevel": "Oberste Ebene",
  "filed.here": "Aktuell hier",
  "filed.destinations": "Verfügbare Ordner",
  "filed.loadError": "Die Ordner konnten nicht geladen werden.",
  "filed.moveFailed": "Verschieben fehlgeschlagen. Versuch es noch einmal.",
  "filed.moveTrack": "„{title}“ in einen Ordner verschieben",
  "filed.moveFolder": "„{name}“ verschieben",
  "filed.note":
    "Ordner, die nur in WaveFlow existieren. Wenn du einen Titel hier einsortierst, wird keine Datei verschoben.",
  "filed.newFolder": "Neuer Ordner",
  "filed.name": "Ordnername",
  "filed.renameLabel": "Neuer Name",
  "filed.create": "Erstellen",
  "filed.save": "Speichern",
  "filed.cancel": "Abbrechen",
  "filed.rename": "Umbenennen",
  "filed.move": "Verschieben…",
  "filed.delete": "Löschen",
  "filed.confirmDelete":
    "„{name}“ und seine Unterordner löschen? Ihre Titel kommen zurück nach „Unsortiert“; keine Datei wird angerührt.",
  "filed.created": "„{name}“ erstellt.",
  "filed.renamed": "Umbenannt in „{name}“.",
  "filed.deleted": {
    one: "Ordner gelöscht: {count} Titel ist zurück in „Unsortiert“.",
    other: "Ordner gelöscht: {count} Titel sind zurück in „Unsortiert“.",
  },
  "filed.filed": {
    one: "{count} Titel in {folder} einsortiert.",
    other: "{count} Titel in {folder} einsortiert.",
  },
  "filed.folderMoved": "„{name}“ verschoben.",
  "filed.nameTaken": "Hier gibt es bereits einen Ordner mit diesem Namen.",
  "filed.refused":
    "Dieser Ordner kann nicht dorthin: Prüf seinen Namen, oder die Struktur ist zu tief oder zu voll.",
  "filed.failed": "Das hat nicht geklappt. Versuch es noch einmal.",
  "filed.missing": "Diesen Ordner gibt es nicht mehr.",
  "filed.empty": "Hier ist noch nichts einsortiert.",
  "filed.allFiled": "Alle Titel sind einsortiert.",
  "filed.uploadHere": "Hierher hochladen",
  "filed.unfiledNote":
    "Alle Titel, die noch in keinem Ordner liegen. WaveFlow verwaltet diese Liste selbst: Ein Titel verlässt sie, sobald er einsortiert ist, und sie kann nicht umbenannt oder gelöscht werden. Um einen Titel einzusortieren, öffne sein Menü und wähle „In Ordner verschieben…“.",
  "filed.unfiledNoteReader":
    "Alle Titel, die noch in keinem Ordner liegen. WaveFlow verwaltet diese Liste selbst: Ein Titel verlässt sie, sobald er einsortiert ist.",
  "upload.into": "Empfangene Dateien kommen in den Ordner {folder}.",
  "upload.doneUnfiled": "Zur Bibliothek hinzugefügt, unter „Unsortiert“",
  "upload.folderMissing":
    "Der Ordner, für den dieser Upload geöffnet wurde, ist nicht auffindbar: Empfangene Dateien landen in „Unsortiert“.",
  "upload.folderUnchecked":
    "Der Ordner, für den dieser Upload geöffnet wurde, konnte nicht geprüft werden, daher wird noch keine Datei angenommen.",
  "upload.checkAgain": "Erneut prüfen",
  "upload.title": "In {library} hochladen",
  "upload.detail":
    "Jede Datei bekommt zuerst in deinem Browser einen Fingerabdruck. So wird eine Datei, die schon in der Bibliothek liegt, nie erneut gesendet, und eine unterbrochene Übertragung setzt dort fort, wo sie aufgehört hat.",
  "upload.pick": "Audiodateien auswählen",
  "upload.formats": "Formate, die die Bibliothek indexieren kann: {extensions}",
  "upload.noLibrary": "Wähle zuerst eine Bibliothek.",
  "upload.notAllowed":
    "Nur Eigentümer oder Verwalter dieser Bibliothek können Dateien hinzufügen.",
  "upload.closed":
    "Diese Bibliothek nimmt keine Dateien an. Das legt ihr Betreiber auf dem Server fest.",
  "upload.waiting": "Wartet",
  "upload.hashing": "Fingerabdruck wird erstellt…",
  "upload.negotiating": "Server wird gefragt…",
  "upload.uploading": "Wird gesendet…",
  "upload.committing": "Wird zur Bibliothek hinzugefügt…",
  "upload.done": "Zur Bibliothek hinzugefügt",
  "upload.present": "Bereits in der Bibliothek",
  "upload.refused": "Abgelehnt: {reason}",
  "upload.failed": "Fehlgeschlagen: {reason}",
  "upload.progress": "Fortschritt: {name}",
  "upload.reason.unsupportedFormat":
    "kein Audioformat, das die Bibliothek indexieren kann",
  "upload.reason.tooLarge": "größer, als der Server annimmt",
  "upload.reason.quotaExceeded": "die Bibliothek hat keinen Platz mehr",
  "upload.reason.libraryClosed": "die Bibliothek nimmt keine Dateien an",
  "upload.reason.tooManySessions":
    "zu viele offene Übertragungen – versuch es gleich noch einmal",
  "upload.reason.refusedFile":
    "das Empfangene passt nicht zur Datei oder ist kein Audio, das die Bibliothek lesen kann",
  "upload.reason.unavailable":
    "der Server konnte es gerade nicht annehmen – versuch es noch einmal",
  "upload.reason.network": "die Verbindung wurde unterbrochen",
  "upload.reason.notAllowed":
    "du darfst dieser Bibliothek keine Dateien mehr hinzufügen",
  "upload.reason.error": "etwas ist schiefgelaufen",
  "upload.reason.hashing":
    "dieser Browser konnte keinen Fingerabdruck der Datei erstellen",
};
