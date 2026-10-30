// Preferences, ReplayGain, the settings page and themes.
import type { TranslationValue } from "..";
import type { preferences as en } from "../en/preferences";

export const preferences: Record<keyof typeof en, TranslationValue> = {
  "preferences.theme": "Tema",
  "preferences.language": "Idioma",
  "preferences.themeLight": "Claro",
  "preferences.themeDark": "Oscuro",
  "preferences.followSystem": "Seguir al sistema",
  "preferences.followSystemDetail":
    "Cambia entre la versión clara y la oscura de un tema cuando lo hace tu dispositivo.",
  "preferences.followingLight":
    "Tu dispositivo está en modo claro, así que se muestra la versión clara. Al elegir un tema eliges sus dos versiones.",
  "preferences.followingDark":
    "Tu dispositivo está en modo oscuro, así que se muestra la versión oscura. Al elegir un tema eliges sus dos versiones.",
  "preferences.standInLight":
    "{theme} no tiene versión clara, así que se usa {fallback} mientras tu dispositivo esté en modo claro.",
  "preferences.standInDark":
    "{theme} no tiene versión oscura, así que se usa {fallback} mientras tu dispositivo esté en modo oscuro.",
  "replayGain.label": "ReplayGain",
  "replayGain.off": "Desactivado",
  "replayGain.track": "Ganancia por pista",
  "replayGain.album": "Ganancia por álbum",
  "replayGain.preamp": "Preamplificación",
  "replayGain.detail":
    "Reproduce cada pista al volumen que midieron sus etiquetas, para que un disco no suene más fuerte que el siguiente. La ganancia por álbum conserva las diferencias dentro de un álbum. Una pista sin etiquetas suena tal cual. La reproducción pasa por Web Audio, que algunos teléfonos pausan al apagar la pantalla.",
  "settings.detail":
    "Apariencia, idioma y los clientes autorizados a leer tu biblioteca.",
  "settings.appearance": "Apariencia",
  "settings.playback": "Reproducción",
  "settings.sections": "Secciones de Ajustes",
  "settings.section.account": "Cuenta",
  "settings.playbackDetail":
    "Cómo reproduce tus pistas este navegador. Se guarda en este navegador, no en tu cuenta.",
  "settings.levelling": "Nivelación del volumen",
  "appearance.themes.default": "Esmeralda claro",
  "appearance.themes.midnightLight": "Medianoche claro",
  "appearance.themes.sunsetLight": "Atardecer claro",
  "appearance.themes.lavenderLight": "Lavanda claro",
  "appearance.themes.crimsonLight": "Carmesí claro",
  "appearance.themes.oceanLight": "Océano claro",
  "appearance.themes.defaultDark": "Esmeralda oscuro",
  "appearance.themes.oled": "Negro OLED",
  "appearance.themes.midnight": "Medianoche",
  "appearance.themes.sunset": "Atardecer",
  "appearance.themes.lavender": "Lavanda",
  "appearance.themes.crimson": "Carmesí",
  "appearance.themes.ocean": "Océano",
  "appearance.themes.neon": "Neón",
};
