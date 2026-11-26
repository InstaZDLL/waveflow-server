// Sign-in and first-run setup.
import type { TranslationValue } from "..";
import type { login as en } from "../en/login";

export const login: Record<keyof typeof en, TranslationValue> = {
  "login.heroEyebrow": "Deine Musik, nach deinen Regeln",
  "login.heroTitle": "Wenn es dunkel wird, erwacht die Bibliothek.",
  "login.heroDetail": "Privates Streaming, ein Katalog, jeder Bildschirm.",
  "login.server": "WaveFlow-Server",
  "login.createAdmin": "Administrator erstellen",
  "login.welcome": "Willkommen zurück",
  "login.setupDetail":
    "Das ist ein neuer Server. Lege das erste Administratorkonto an.",
  "login.setupToken": "Einrichtungstoken",
  "login.setupTokenHint":
    "Der Server hat es in die Datei setup-token in seinem Datenverzeichnis geschrieben, und sein Log nennt den Pfad. Wenn du WAVEFLOW_SETUP_TOKEN gesetzt hast, gib diesen Wert ein.",
  "login.setupTokenError":
    "Dieses Einrichtungstoken ist nicht das richtige. Kopiere es noch einmal aus der Datei setup-token.",
  "login.username": "Benutzername",
  "login.password": "Passwort",
  "login.setupError":
    "Die Einrichtung ist fehlgeschlagen. Verwende einen gültigen Benutzernamen und ein Passwort mit mindestens 12 Zeichen.",
  "login.error": "Falscher Benutzername oder falsches Passwort.",
  "login.originError":
    "Dieser Server akzeptiert keine Anmeldung von {origin}. Öffne ihn unter der Adresse aus WAVEFLOW_PUBLIC_URL, oder korrigiere diese Einstellung.",
  "login.wait": "Bitte warten…",
  "login.create": "Erstellen und anmelden",
  "login.signIn": "Anmelden",
  "login.offline": {
    one: "Den {count} in diesem Browser gespeicherten Titel anhören",
    other: "Die {count} in diesem Browser gespeicherten Titel anhören",
  },
  "login.setupTokenDocker":
    "Mit Docker Compose gibt dieser Befehl das vom Server erzeugte Token aus; ein in WAVEFLOW_SETUP_TOKEN gesetzter Wert steht nicht in dieser Datei:",
};
