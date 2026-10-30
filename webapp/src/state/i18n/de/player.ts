// The player bar, now playing, radio and offline listening.
import type { TranslationValue } from "..";
import type { player as en } from "../en/player";

export const player: Record<keyof typeof en, TranslationValue> = {
  "player.error":
    "Die Wiedergabe konnte nicht starten. Prüf den Server und versuch es noch einmal.",
  "player.previous": "Vorheriger Titel",
  "player.play": "Abspielen",
  "player.pause": "Pause",
  "player.next": "Nächster Titel",
  "player.seek": "Spulen",
  "player.shuffle": "Zufallswiedergabe",
  "player.repeatOff": "Wiederholen: aus",
  "player.repeatAll": "Wiederholen: ganze Warteschlange",
  "player.repeatOne": "Wiederholen: dieser Titel",
  "player.volume": "Lautstärke",
  "player.mute": "Stummschalten",
  "player.unmute": "Ton an",
  "player.openAlbum": "Album öffnen",
  "player.live": "Live",
  "player.liveRadio": "Liveradio",
  "player.stopLive": "Sender stoppen",
  "player.liveError": "Dieser Sender konnte nicht abgespielt werden.",
  "player.closeAside": "Stoppen und schließen",
  "playing.idle": "Gerade läuft nichts.",
  "playing.empty":
    "Starte einen Titel, dann erscheint er hier – mit seinem Songtext.",
  "playing.viewLabel": "Ansicht für diesen Titel",
  "playing.viewDetail": "Details",
  "playing.viewLyrics": "Songtext",
  "playing.viewImmersive": "Vollbild",
  "playing.immersiveTitle": "Vollbildansicht",
  "playing.leaveImmersive": "Vollbildansicht verlassen",
  "radio.detail": "Internetradiosender, in deinem Konto gespeichert",
  "radio.name": "Sendername",
  "radio.address": "Stream-Adresse",
  "radio.homepage": "Website des Senders (optional)",
  "radio.image": "Senderbild (optional)",
  "radio.imageHint":
    "Der Server lädt es beim Speichern einmal herunter und bewahrt es bei den Covern auf: Dein Browser kontaktiert den Host des Bildes nie.",
  "radio.imageMissing":
    "Das Bild konnte nicht abgerufen werden; speichere den Sender, um es erneut zu versuchen.",
  "radio.add": "Sender hinzufügen",
  "radio.save": "Sender speichern",
  "radio.edit": "Bearbeiten",
  "radio.replacing":
    "Beim Speichern werden alle Felder ersetzt, eine leere Website-Adresse löscht also die gespeicherte.",
  "radio.listen": "Anhören",
  "radio.stop": "Stoppen",
  "radio.visit": "Website des Senders",
  "radio.empty":
    "Noch keine Sender. Füge einen hinzu, um ihn hier zu speichern.",
  "radio.addressRefused":
    "Eine Adresse muss mit http:// oder https:// beginnen.",
  "radio.mixedContent":
    "Diese Seite wird über HTTPS ausgeliefert, daher lehnt dein Browser einen unverschlüsselten Stream ab.",
  "radio.full": "Dieses Konto hat bereits so viele Sender, wie es haben darf.",
  "radio.saveError": "Der Sender konnte nicht gespeichert werden.",
  "radio.deleteError": "Der Sender konnte nicht gelöscht werden.",
  "radio.playbackError":
    "Dieser Sender konnte nicht abgespielt werden. Der Server verbindet sich nie mit ihm, das liegt also zwischen deinem Browser und dem Sender.",
  "radio.stations": "Sender",
  "radio.newTitle": "Sender hinzufügen",
  "radio.editTitle": "„{name}“ bearbeiten",
  "radio.listenTo": "{name} anhören",
  "radio.stopNamed": "{name} stoppen",
  "radio.actions": "Aktionen für {name}",
  "radio.confirmDelete":
    "Den Sender „{name}“ löschen? Seine Adresse und sein Bild werden vergessen.",
  "offline.kicker": "In diesem Browser",
  "offline.title": "Offline verfügbar",
  "offline.detail":
    "Diese Titel wurden in diesem Browser für das zuletzt angemeldete Konto gespeichert und laufen ohne Verbindung. Beim Abmelden werden sie aus diesem Browser gelöscht. Weitere Titel speicherst du über ihr Menü.",
  "offline.unsupported":
    "Dieser Browser kann keine Titel zum Offline-Hören speichern.",
  "offline.back": "Zurück zur Bibliothek",
  "offline.empty":
    "Noch nichts gespeichert. Im Menü eines Titels kannst du ihn speichern.",
  "offline.player": "Offline-Player",
  "offline.gone": "Diese Kopie ist nicht mehr in diesem Browser.",
  "offline.forget": "Offline-Kopie von {title} entfernen",
  "offline.forgetShort": "Entfernen",
  "offline.forgetDetail": "Gibt {size} frei",
};
