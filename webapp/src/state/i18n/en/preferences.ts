// Preferences, ReplayGain, the settings page and themes.
export const preferences = {
  "preferences.theme": "Theme",
  "preferences.language": "Language",
  "preferences.themeLight": "Light",
  "preferences.themeDark": "Dark",
  "preferences.followSystem": "Follow the system",
  "preferences.followSystemDetail":
    "Switch between a theme’s light and dark versions when your device does.",
  "preferences.followingLight":
    "Your device is in light mode, so the light version is shown. Picking a theme picks both of its versions.",
  "preferences.followingDark":
    "Your device is in dark mode, so the dark version is shown. Picking a theme picks both of its versions.",
  "preferences.standInLight":
    "{theme} has no light version, so {fallback} stands in while your device is in light mode.",
  "preferences.standInDark":
    "{theme} has no dark version, so {fallback} stands in while your device is in dark mode.",
  "replayGain.label": "ReplayGain",
  "replayGain.off": "Off",
  "replayGain.track": "Track gain",
  "replayGain.album": "Album gain",
  "replayGain.preamp": "Preamp",
  "replayGain.detail":
    "Plays every track at the loudness its tags measured, so one record is not louder than the next. Album gain keeps the differences inside an album. A track without tags plays as it is. It routes playback through Web Audio, which some phones pause with the screen.",
  "settings.detail":
    "Appearance, language, and the clients allowed to read your library.",
  "settings.appearance": "Appearance",
  "settings.playback": "Playback",
  "settings.sections": "Settings sections",
  "settings.section.account": "Account",
  "settings.playbackDetail":
    "How this browser plays your tracks. Kept in this browser, not on your account.",
  "settings.levelling": "Volume levelling",
  "appearance.themes.default": "Emerald light",
  "appearance.themes.midnightLight": "Midnight light",
  "appearance.themes.sunsetLight": "Sunset light",
  "appearance.themes.lavenderLight": "Lavender light",
  "appearance.themes.crimsonLight": "Crimson light",
  "appearance.themes.oceanLight": "Ocean light",
  "appearance.themes.defaultDark": "Emerald dark",
  "appearance.themes.oled": "OLED black",
  "appearance.themes.midnight": "Midnight",
  "appearance.themes.sunset": "Sunset",
  "appearance.themes.lavender": "Lavender",
  "appearance.themes.crimson": "Crimson",
  "appearance.themes.ocean": "Ocean",
  "appearance.themes.neon": "Neon",
} as const;
