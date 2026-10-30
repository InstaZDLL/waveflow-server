// Administration, scans, maintenance and missing files.
import type { TranslationValue } from "..";
import type { admin as en } from "../en/admin";

export const admin: Record<keyof typeof en, TranslationValue> = {
  "admin.title": "Administration",
  "admin.detail": "Bibliotheken, Scans und Konten",
  "admin.libraries": "Bibliotheken",
  "admin.libraryName": "Name der Bibliothek",
  "admin.libraryPath": "Absoluter Ordnerpfad auf dem Server",
  "admin.registering": "Wird angelegt…",
  "admin.register": "Anlegen und scannen",
  "admin.scan": "Jetzt scannen",
  "admin.accounts": "Konten",
  "admin.webPassword": "Web-Passwort, mindestens 12 Zeichen",
  "admin.creating": "Wird erstellt…",
  "admin.create": "Konto erstellen",
  "admin.enable": "Aktivieren",
  "admin.disable": "Deaktivieren",
  "admin.currentAccount": "Du kannst dein aktuelles Konto nicht deaktivieren",
  "admin.libraryError": "Der Bibliothekspfad konnte nicht angelegt werden.",
  "admin.accountError": "Das Konto konnte nicht erstellt werden.",
  "admin.statusError": "Der Kontostatus konnte nicht geändert werden.",
  "admin.scanError": "Der Scan konnte nicht gestartet werden.",
  "admin.scanQueued": "Scan {id} ist eingereiht.",
  "admin.initialScan": "Erster Scan {id} ist eingereiht.",
  "admin.scanning": "Scan läuft",
  "admin.nowPlaying": "Läuft gerade",
  "admin.nobodyListening": "Gerade hört niemand zu.",
  "admin.tokens": "API-Token für {username}",
  "admin.tokenName": "Wofür ist es?",
  "admin.tokenCreate": "Ausstellen",
  "admin.tokenOnce": "Kopiere es jetzt. Es wird nur dieses eine Mal angezeigt:",
  "admin.tokenError": "Das Token konnte nicht ausgestellt werden.",
  "admin.tokenRevoke": "Widerrufen",
  "admin.tokenUsed": "bereits verwendet",
  "admin.tokenUnused": "nie verwendet",
  "admin.tokenNone": "Kein Token für dieses Konto.",
  "admin.members": "Wer diese Bibliothek sehen darf",
  "admin.memberRole": "Rolle",
  "admin.memberRemove": "Entfernen",
  "admin.memberOwner": "Eigentümer, und bleibt es",
  "admin.memberAdd": "Zugriff geben für",
  "admin.memberChoose": "Konto auswählen…",
  "admin.memberError": "Diese Mitgliedschaft konnte nicht geändert werden.",
  "admin.roleAdmin": "Administrator",
  "admin.roleUser": "Benutzer",
  "admin.roleOwner": "Eigentümer",
  "admin.roleManager": "Verwalter",
  "admin.roleListener": "Hörer",
  "admin.accountLibraries": {
    one: "{count} Bibliothek",
    other: "{count} Bibliotheken",
  },
  "admin.manage": "Name, Ordner und Löschen",
  "admin.rename": "Name",
  "admin.path": "Ordner auf dem Server",
  "admin.followLinks":
    "Symbolischen Links folgen, deren Ziel innerhalb der Bibliothek bleibt",
  "admin.watch":
    "Ordner überwachen und Änderungen scannen (aus bei einer Netzwerkfreigabe, die von anderen Rechnern geändert wird)",
  "admin.save": "Speichern",
  "admin.saved": "Gespeichert.",
  "admin.savedScan": "Gespeichert. Scan {id} ist eingereiht.",
  "admin.saveError":
    "Die Änderung konnte nicht gespeichert werden. Der Ordner muss auf dem Server existieren und darf kein symbolischer Link sein.",
  "admin.saveBusy":
    "Die Bibliothek wird gerade gescannt, oder eine andere Bibliothek nutzt diesen Ordner bereits. Versuch es nach dem Scan noch einmal.",
  "admin.delete": "Diese Bibliothek löschen",
  "admin.deleteNote":
    "Entfernt die Bibliothek für alle Mitglieder aus dem Katalog: ihre Titel, Alben und Künstler samt Playlist-Einträgen, Favoriten und Wiedergaben. Keine Datei auf der Festplatte wird angerührt. Gib zur Bestätigung den Namen der Bibliothek ein.",
  "admin.deleteConfirm": "Name der Bibliothek, zur Bestätigung",
  "admin.deleteButton": "Endgültig löschen",
  "admin.deleted":
    "{name} wurde gelöscht: {tracks} Titel sind aus dem Katalog verschwunden.",
  "admin.deleteError": "Die Bibliothek konnte nicht gelöscht werden.",
  "scan.connecting": "Warte auf den ersten Messwert…",
  "scan.queued": "Eingereiht",
  "scan.running": "Wird gescannt",
  "scan.completed": "Fertig",
  "scan.failed": "Fehlgeschlagen",
  "scan.progress": "Scan-Fortschritt",
  "scan.counted": "{processed} von {total} Dateien",
  "scan.tally":
    "{added} hinzugefügt · {updated} aktualisiert · {moved} verschoben · {skipped} unverändert · {errors} Fehler",
  "scan.lost":
    "Der Fortschritt konnte nicht abgerufen werden. Der Scan selbst läuft normal weiter.",
  "scan.leftOut": "{excluded} ausgelassen · {links} Links nicht gefolgt",
  "maintenance.sections": "Wartungsbereiche",
  "maintenance.none": "In dieser Bibliothek gibt es für dich nichts zu warten.",
  "missing.total": {
    one: "{count} Datei, die die Scans nicht mehr finden",
    other: "{count} Dateien, die die Scans nicht mehr finden",
  },
  "missing.nothing": "Jede Datei liegt dort, wo der Katalog sie erwartet.",
  "missing.notYours":
    "Nur Eigentümer und Verwalter dieser Bibliothek sowie Administratoren sehen ihre fehlenden Dateien.",
  "missing.note":
    "Diese Titel bleiben samt Favoriten, Playlist-Einträgen und Wiedergaben im Katalog, bis sie endgültig entfernt werden; taucht eine Datei vorher wieder auf, behält sie ihren Titel. Das Entfernen lässt sich eine Zeit lang rückgängig machen, danach verschwindet der Titel endgültig – mit allem, was für jedes Mitglied daran hängt.",
  "missing.since": "fehlt seit {date}",
  "missing.cost":
    "{playlists} in Playlists · {favorites} Favoriten · {plays} Wiedergaben",
  "missing.select": "{title} auswählen",
  "missing.purgeSelected": "Auswahl endgültig entfernen ({count})",
  "missing.purgeAll": "Alle endgültig entfernen",
  "missing.confirm": "Bestätigen: {count} endgültig entfernen",
  "missing.purged": {
    one: "{count} Titel endgültig entfernt.",
    other: "{count} Titel endgültig entfernt.",
  },
  "missing.scanning":
    "Die Bibliothek wird gerade gescannt. Versuch es nach dem Scan noch einmal.",
  "missing.failed": "Das Entfernen hat nicht geklappt. Versuch es noch einmal.",
  "missing.more": "Mehr anzeigen",
  "missing.purgedUntil": {
    one: "{count} Titel endgültig entfernt. Er kann bis {date} wiederhergestellt werden.",
    other:
      "{count} Titel endgültig entfernt. Sie können bis {date} wiederhergestellt werden.",
  },
  "missing.purgedTitle": {
    one: "Entfernt, noch wiederherstellbar ({count})",
    other: "Entfernt, noch wiederherstellbar ({count})",
  },
  "missing.purgedNote":
    "Davon ist noch nichts verloren. Ein wiederhergestellter Titel kehrt mit allem, was an ihm hing, zu den fehlenden Dateien zurück; taucht eine Datei wieder auf, stellt sie ihren Titel von selbst wieder her.",
  "missing.deletedAfter": "endgültig gelöscht nach {date}",
  "missing.restore": "Wiederherstellen",
  "missing.restoreOne": "{title} wiederherstellen",
  "missing.restoreAll": "Alle wiederherstellen",
  "missing.restored": {
    one: "{count} Titel wiederhergestellt.",
    other: "{count} Titel wiederhergestellt.",
  },
  "missing.restoreFailed":
    "Das Wiederherstellen hat nicht geklappt. Versuch es noch einmal.",
};
