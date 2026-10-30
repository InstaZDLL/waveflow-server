// Preferences, ReplayGain, the settings page and themes.
import type { TranslationValue } from "..";
import type { preferences as en } from "../en/preferences";

export const preferences: Record<keyof typeof en, TranslationValue> = {
  "preferences.theme": "Tema",
  "preferences.language": "Lingua",
  "preferences.themeLight": "Chiaro",
  "preferences.themeDark": "Scuro",
  "preferences.followSystem": "Segui il sistema",
  "preferences.followSystemDetail":
    "Passa dalla versione chiara a quella scura di un tema quando lo fa il tuo dispositivo.",
  "preferences.followingLight":
    "Il tuo dispositivo è in modalità chiara, quindi viene mostrata la versione chiara. Scegliendo un tema ne scegli entrambe le versioni.",
  "preferences.followingDark":
    "Il tuo dispositivo è in modalità scura, quindi viene mostrata la versione scura. Scegliendo un tema ne scegli entrambe le versioni.",
  "preferences.standInLight":
    "{theme} non ha una versione chiara, quindi al suo posto viene usato {fallback} mentre il tuo dispositivo è in modalità chiara.",
  "preferences.standInDark":
    "{theme} non ha una versione scura, quindi al suo posto viene usato {fallback} mentre il tuo dispositivo è in modalità scura.",
  "replayGain.label": "ReplayGain",
  "replayGain.off": "No",
  "replayGain.track": "Guadagno del brano",
  "replayGain.album": "Guadagno dell’album",
  "replayGain.preamp": "Preamplificazione",
  "replayGain.detail":
    "Riproduce ogni brano al volume misurato nei suoi tag, così un disco non suona più forte del successivo. Il guadagno dell’album conserva le differenze all’interno di un album. Un brano senza tag viene riprodotto così com’è. La riproduzione passa da Web Audio, che alcuni telefoni mettono in pausa quando lo schermo si spegne.",
  "settings.detail":
    "Aspetto, lingua e i client autorizzati a leggere la tua libreria.",
  "settings.appearance": "Aspetto",
  "settings.playback": "Riproduzione",
  "settings.sections": "Sezioni delle impostazioni",
  "settings.section.account": "Account",
  "settings.playbackDetail":
    "Come questo browser riproduce i tuoi brani. Salvato in questo browser, non nel tuo account.",
  "settings.levelling": "Normalizzazione del volume",
  "appearance.themes.default": "Smeraldo chiaro",
  "appearance.themes.midnightLight": "Mezzanotte chiaro",
  "appearance.themes.sunsetLight": "Tramonto chiaro",
  "appearance.themes.lavenderLight": "Lavanda chiaro",
  "appearance.themes.crimsonLight": "Cremisi chiaro",
  "appearance.themes.oceanLight": "Oceano chiaro",
  "appearance.themes.defaultDark": "Smeraldo scuro",
  "appearance.themes.oled": "Nero OLED",
  "appearance.themes.midnight": "Mezzanotte",
  "appearance.themes.sunset": "Tramonto",
  "appearance.themes.lavender": "Lavanda",
  "appearance.themes.crimson": "Cremisi",
  "appearance.themes.ocean": "Oceano",
  "appearance.themes.neon": "Neon",
};
