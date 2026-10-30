// Registered clients, the Subsonic password and OAuth consent.
import type { TranslationValue } from "..";
import type { clients as en } from "../en/clients";

export const clients: Record<keyof typeof en, TranslationValue> = {
  "clients.title": "Registrierte Clients",
  "clients.detail":
    "Geräte und Skripte mit Zugangsdaten zu deiner Bibliothek. Ein Widerruf sperrt sie sofort.",
  "clients.name": "Wofür es ist",
  "clients.register": "Registrieren",
  "clients.once":
    "Kopiere es jetzt. Gespeichert wird nur sein Hash, daher wird es nur dieses eine Mal angezeigt:",
  "clients.revoke": "Widerrufen",
  "clients.none": "Kein Client hat Zugangsdaten zu deiner Bibliothek.",
  "clients.error": "Das hat nicht geklappt. Nichts wurde geändert.",
  "clients.registered": "registriert {when}",
  "clients.used": "zuletzt verwendet {when}",
  "clients.unused": "nie verwendet",
  "subsonic.title": "Subsonic-Passwort",
  "subsonic.detail":
    "Das separate Passwort, mit dem sich Subsonic-Clients anmelden. Wenn du es änderst, wird auch der API-Schlüssel ersetzt, sodass ein geleakter nicht mehr funktioniert.",
  "subsonic.set": "festgelegt {when}",
  "subsonic.changed": "zuletzt geändert {when}",
  "subsonic.password": "Neues Passwort",
  "subsonic.rotate": "Ändern",
  "subsonic.revoke": "Abschalten",
  "subsonic.confirm": "Ja, abschalten",
  "subsonic.once":
    "Kopiere diesen API-Schlüssel jetzt. Gespeichert wird nur sein Hash, daher wird er nur dieses eine Mal angezeigt:",
  "subsonic.gone":
    "Deine Subsonic-Clients funktionieren erst wieder, wenn du es in jedem von ihnen neu einträgst.",
  "subsonic.none":
    "Du hast kein Subsonic-Passwort, daher kann kein Subsonic-Client deine Bibliothek lesen. Frag den Administrator nach einem.",
  "subsonic.short": "Verwende mindestens 12 Zeichen.",
  "subsonic.error": "Das hat nicht geklappt. Nichts wurde geändert.",
  "credential.password": "Eigenes Subsonic-Passwort",
  "credential.rotate": "Zugangsdaten erneuern",
  "credential.copy": "Kopiere diesen API-Schlüssel jetzt:",
  "credential.error": "Die Zugangsdaten konnten nicht erneuert werden.",
  "authorize.title": "Autorisierung",
  "authorize.invalid":
    "Dieser Autorisierungslink ist unvollständig oder will dich an eine Adresse schicken, der wir nicht vertrauen.",
  "authorize.client": "{client} autorisieren",
  "authorize.detail":
    "Die App kann deine Bibliotheken durchsuchen, deine Musik abspielen und deine Playlists, Favoriten und Bewertungen verwalten – alles, was dieses Konto kann.",
  "authorize.redirect": "Du wirst zurückgeleitet zu {url}",
  "authorize.error":
    "Die Anwendung hat eine Autorisierungsanfrage gesendet, die wir nicht erfüllen können.",
  "authorize.busy": "Wird autorisiert…",
  "authorize.approve": "Autorisieren",
};
