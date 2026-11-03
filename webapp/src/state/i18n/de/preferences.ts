// Preferences, ReplayGain, the settings page and themes.
import type { TranslationValue } from "..";
import type { preferences as en } from "../en/preferences";

export const preferences: Record<keyof typeof en, TranslationValue> = {
  "preferences.theme": "Design",
  "preferences.language": "Sprache",
  "preferences.themeLight": "Hell",
  "preferences.themeDark": "Dunkel",
  "preferences.followSystem": "Systemeinstellung folgen",
  "preferences.followSystemDetail":
    "Je nach Systemeinstellung deines Geräts zwischen der hellen und dunklen Variante eines Designs wechseln.",
  "preferences.followingLight":
    "Dein Gerät ist im hellen Modus, daher wird die helle Variante angezeigt. Wenn du ein Design wählst, wählst du beide Varianten.",
  "preferences.followingDark":
    "Dein Gerät ist im dunklen Modus, daher wird die dunkle Variante angezeigt. Wenn du ein Design wählst, wählst du beide Varianten.",
  "preferences.standInLight":
    "{theme} hat keine helle Variante, daher springt {fallback} ein, solange dein Gerät im hellen Modus ist.",
  "preferences.standInDark":
    "{theme} hat keine dunkle Variante, daher springt {fallback} ein, solange dein Gerät im dunklen Modus ist.",
  "replayGain.label": "ReplayGain",
  "replayGain.off": "Aus",
  "replayGain.track": "Titelpegel",
  "replayGain.album": "Albumpegel",
  "replayGain.preamp": "Vorverstärkung",
  "replayGain.detail":
    "Gleicht die Lautstärke anhand der Werte in den Tags an, damit ein Album nicht lauter ist als das nächste. Der Albumpegel erhält die Unterschiede innerhalb eines Albums. Ein Titel ohne ReplayGain-Tags läuft unverändert. Die Wiedergabe läuft dann über Web Audio, das manche Smartphones bei ausgeschaltetem Bildschirm pausieren.",
  "settings.detail":
    "Darstellung, Sprache und die Clients, die deine Bibliothek lesen dürfen.",
  "settings.appearance": "Darstellung",
  "settings.playback": "Wiedergabe",
  "settings.sections": "Bereiche der Einstellungen",
  "settings.section.account": "Konto",
  "settings.playbackDetail":
    "Wie deine Titel abgespielt werden. Der Lautstärkeausgleich wird in diesem Browser gespeichert, die Transkodierung in deinem Konto.",
  "settings.levelling": "Lautstärkeausgleich",
  "transcoding.title": "Transkodierung für deine Apps",
  "transcoding.detail":
    "Wird in deinem Konto gespeichert. Für Apps, die selbst kein Format und keine Bitrate anfordern: Eine App, die etwas anfordert, bekommt immer genau das. Ein Titel, der die Bitrate nicht überschreitet, wird unverändert abgespielt. Dieser Webplayer spielt weiterhin die Originale.",
  "transcoding.seeking":
    "Solange ein Titel noch umgewandelt wird, können Apps, die anhand der Dateiposition spulen, dabei Probleme haben. Wenn das bei einer deiner Apps passiert, füge sie unten mit „Original“ hinzu.",
  "transcoding.allApps": "Alle Apps",
  "transcoding.format": "Format",
  "transcoding.original": "Original",
  "transcoding.bitrate": "Höchste Bitrate (kbit/s)",
  "transcoding.bitrateDefault": "Standard des Formats",
  "transcoding.save": "Speichern",
  "transcoding.saved": "Gespeichert.",
  "transcoding.oneApp": "Für eine App",
  "transcoding.oneAppDetail":
    "Ersetzt die obige Einstellung für diese App. Verwende den Namen, mit dem sie sich beim Server meldet. Vorgeschlagen werden Apps, die hier bereits Musik abgespielt haben, ohne selbst ein Format oder eine Bitrate anzufordern.",
  "transcoding.client": "Name der App",
  "transcoding.add": "Hinzufügen",
  "transcoding.remove": "Entfernen",
  "transcoding.none": "Keine App hat eine eigene Einstellung.",
  "transcoding.rate": "{format} · {bitrate} kbit/s",
  "transcoding.bitrateRange":
    "Die Bitrate muss zwischen 32 und 512 kbit/s liegen.",
  "transcoding.error":
    "Das konnte nicht gespeichert werden, es wurde nichts geändert.",
  "appearance.themes.default": "Smaragd hell",
  "appearance.themes.midnightLight": "Mitternacht hell",
  "appearance.themes.sunsetLight": "Abendrot hell",
  "appearance.themes.lavenderLight": "Lavendel hell",
  "appearance.themes.crimsonLight": "Karmesin hell",
  "appearance.themes.oceanLight": "Ozean hell",
  "appearance.themes.defaultDark": "Smaragd dunkel",
  "appearance.themes.oled": "OLED-Schwarz",
  "appearance.themes.midnight": "Mitternacht",
  "appearance.themes.sunset": "Abendrot",
  "appearance.themes.lavender": "Lavendel",
  "appearance.themes.crimson": "Karmesin",
  "appearance.themes.ocean": "Ozean",
  "appearance.themes.neon": "Neon",
};
