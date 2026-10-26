// Preferences, ReplayGain, the settings page and themes.
import type { TranslationValue } from "..";
import type { preferences as en } from "../en/preferences";

export const preferences: Record<keyof typeof en, TranslationValue> = {
  "preferences.theme": "Thème",
  "preferences.language": "Langue",
  "preferences.themeLight": "Clair",
  "preferences.themeDark": "Sombre",
  "preferences.followSystem": "Suivre le système",
  "preferences.followSystemDetail":
    "Passer de la version claire d’un thème à sa version sombre en même temps que votre appareil.",
  "preferences.followingLight":
    "Votre appareil est en mode clair : la version claire est affichée. Choisir un thème choisit ses deux versions.",
  "preferences.followingDark":
    "Votre appareil est en mode sombre : la version sombre est affichée. Choisir un thème choisit ses deux versions.",
  "preferences.standInLight":
    "{theme} n’a pas de version claire : {fallback} le remplace tant que votre appareil est en mode clair.",
  "preferences.standInDark":
    "{theme} n’a pas de version sombre : {fallback} le remplace tant que votre appareil est en mode sombre.",
  "replayGain.label": "ReplayGain",
  "replayGain.off": "Désactivé",
  "replayGain.track": "Gain par piste",
  "replayGain.album": "Gain par album",
  "replayGain.preamp": "Préampli",
  "replayGain.detail":
    "Joue chaque piste au volume mesuré dans ses tags, pour qu’un disque ne sonne pas plus fort que le suivant. Le gain par album garde les écarts à l’intérieur d’un album. Une piste sans tags est jouée telle quelle. La lecture passe alors par Web Audio, que certains téléphones mettent en pause avec l’écran.",
  "settings.detail":
    "Apparence, langue, et les clients autorisés à lire votre bibliothèque.",
  "settings.appearance": "Apparence",
  "settings.playback": "Lecture",
  "settings.sections": "Sections des réglages",
  "settings.section.account": "Compte",
  "settings.playbackDetail":
    "Comment ce navigateur lit vos titres. Gardé dans ce navigateur, pas sur votre compte.",
  "settings.levelling": "Normalisation du volume",
  "appearance.themes.default": "Émeraude clair",
  "appearance.themes.midnightLight": "Minuit clair",
  "appearance.themes.sunsetLight": "Coucher de soleil clair",
  "appearance.themes.lavenderLight": "Lavande clair",
  "appearance.themes.crimsonLight": "Cramoisi clair",
  "appearance.themes.oceanLight": "Océan clair",
  "appearance.themes.defaultDark": "Émeraude sombre",
  "appearance.themes.oled": "Noir OLED",
  "appearance.themes.midnight": "Minuit",
  "appearance.themes.sunset": "Coucher de soleil",
  "appearance.themes.lavender": "Lavande",
  "appearance.themes.crimson": "Cramoisi",
  "appearance.themes.ocean": "Océan",
  "appearance.themes.neon": "Néon",
};
