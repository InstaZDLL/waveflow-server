// Playlists and their import, the queue, shares and bookmarks.
import type { TranslationValue } from "..";
import type { playlists as en } from "../en/playlists";

export const playlists: Record<keyof typeof en, TranslationValue> = {
  "playlists.detail": {
    one: "{count} Sammlung",
    other: "{count} Sammlungen",
  },
  "playlists.newName": "Name der neuen Playlist",
  "playlists.create": "Playlist erstellen",
  "playlists.play": "Abspielen",
  "playlists.addQueue": "Warteschlange hinzufügen",
  "playlists.emptyOne": "Diese Playlist ist leer.",
  "playlists.export": "Als .m3u8 speichern",
  "playlists.exportError": "Diese Playlist konnte nicht gespeichert werden.",
  "playlists.empty":
    "Erstelle eine Playlist, um eine Auswahl von Titeln zusammenzuhalten.",
  "playlists.createError": "Die Playlist konnte nicht erstellt werden.",
  "playlists.queueError":
    "Die Warteschlange konnte dieser Playlist nicht hinzugefügt werden.",
  "playlists.deleteError": "Die Playlist konnte nicht gelöscht werden.",
  "playlists.public": "Für alle Konten sichtbar",
  "playlists.publicError": "Diese Playlist konnte nicht veröffentlicht werden.",
  "playlists.others": "Von anderen Konten",
  "playlists.othersDetail":
    "Öffentliche Playlists anderer Konten, mit den Titeln, die deine Bibliotheken enthalten.",
  "playlists.by": "Von {owner}",
  "playlists.moveUp": "{title} nach oben verschieben",
  "playlists.moveDown": "{title} nach unten verschieben",
  "playlists.removeEntry": "{title} aus dieser Playlist entfernen",
  "playlists.moveError":
    "Dieser Eintrag konnte nicht verschoben werden. Die Playlist wurde vielleicht anderswo geändert; sie wurde neu geladen.",
  "playlists.removeEntryError": "Dieser Eintrag konnte nicht entfernt werden.",
  "import.title": "Playlists aus einem anderen Player übernehmen",
  "import.detail":
    "Eine .m3u- oder .m3u8-Datei. Es wird nichts erstellt, bevor du gesehen hast, was dabei verloren ginge.",
  "import.pick": "Playlist-Datei auswählen",
  "import.name": "Name der Playlist",
  "import.found":
    "{resolved} von {entries} Einträgen sind in deiner Bibliothek.",
  "import.encoding":
    "Die Datei war nicht in UTF-8 und wurde als {encoding} gelesen. Prüf die Namen unten, bevor du importierst.",
  "import.missing": {
    one: "{count} Eintrag lässt sich nicht zuordnen:",
    other: "{count} Einträge lassen sich nicht zuordnen:",
  },
  "import.line": "Zeile {line}",
  "import.noMatch": "kein passender Titel",
  "import.ambiguous": "passt zu mehreren Titeln",
  "import.notAPath": "keine Datei in einer Bibliothek",
  "import.duplicate": "bereits in dieser Playlist",
  "import.confirm": "Importieren",
  "import.cancel": "Abbrechen",
  "import.failed": "Diese Datei konnte nicht als Playlist gelesen werden.",
  "queue.detail": {
    one: "{count} Titel, mit deinem Konto synchronisiert",
    other: "{count} Titel, mit deinem Konto synchronisiert",
  },
  "queue.playStart": "Von vorn abspielen",
  "queue.clear": "Warteschlange leeren",
  "queue.remove": "Entfernen",
  "queue.empty":
    "Spiel ein Album oder eine Playlist ab, um deine synchronisierte Warteschlange zu füllen.",
  "shares.detail": "Öffentliche Links für ausgewählte Musik",
  "shares.description": "Beschreibung",
  "shares.create": "Aktuelle Warteschlange teilen",
  "shares.needQueue":
    "Füge der Warteschlange Titel hinzu, bevor du einen Link erstellst.",
  "shares.once": "Dieser Link wird nur einmal angezeigt:",
  "shares.defaultName": "Geteilte Musik",
  "shares.empty": "Es wurden noch keine öffentlichen Links erstellt.",
  "shares.createError": "Die Freigabe konnte nicht erstellt werden.",
  "shares.tooLong":
    "Eine Freigabe umfasst bis zu {limit} Titel, die Warteschlange enthält {count}. Kürze die Warteschlange, um sie zu teilen.",
  "shares.deleteError": "Die Freigabe konnte nicht gelöscht werden.",
  "shares.maxVisits": "Besuchslimit (optional)",
  "shares.visits": { one: "{count} Besuch", other: "{count} Besuche" },
  "shares.visitsOf": "{count} von {max} Besuchen",
  "share.kicker": "Mit dir geteilt",
  "share.defaultTitle": "Mit dir geteilte Musik",
  "share.until": "Verfügbar bis {date}",
  "share.empty": "Keiner dieser Titel kann mehr abgespielt werden.",
  "share.player": "Player",
  "share.unavailableTitle": "Dieser Link führt ins Leere",
  "share.unavailableDetail":
    "Er ist vielleicht abgelaufen, wurde zurückgezogen oder so oft geöffnet, wie er erlaubt.",
  "bookmarks.title": "Gespeicherte Positionen",
  "bookmarks.detail": "Eine pro Titel. Erneutes Speichern verschiebt sie.",
  "bookmarks.save": "Diese Position speichern",
  "bookmarks.forget": "Vergessen",
};
