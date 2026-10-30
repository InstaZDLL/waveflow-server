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
    "Zwischen der hellen und dunklen Variante eines Designs wechseln, wenn dein Gerät es tut.",
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
    "Spielt jeden Titel in der Lautheit ab, die seine Tags gemessen haben, damit ein Album nicht lauter ist als das nächste. Der Albumpegel erhält die Unterschiede innerhalb eines Albums. Ein Titel ohne Tags läuft unverändert. Die Wiedergabe läuft dann über Web Audio, das manche Smartphones bei ausgeschaltetem Bildschirm pausieren.",
  "settings.detail":
    "Darstellung, Sprache und die Clients, die deine Bibliothek lesen dürfen.",
  "settings.appearance": "Darstellung",
  "settings.playback": "Wiedergabe",
  "settings.sections": "Bereiche der Einstellungen",
  "settings.section.account": "Konto",
  "settings.playbackDetail":
    "Wie dieser Browser deine Titel abspielt. Wird in diesem Browser gespeichert, nicht in deinem Konto.",
  "settings.levelling": "Lautstärkeausgleich",
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
