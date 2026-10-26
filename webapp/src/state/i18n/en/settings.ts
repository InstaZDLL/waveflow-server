// Settings, registered clients, the Subsonic password and quality labels.
export const settings = {
  "nav.settings": "Settings",
  "settings.detail":
    "Appearance, language, and the clients allowed to read your library.",
  "settings.appearance": "Appearance",
  "settings.playback": "Playback",
  "settings.sections": "Settings sections",
  "settings.section.account": "Account",
  "settings.playbackDetail":
    "How this browser plays your tracks. Kept in this browser, not on your account.",
  "settings.levelling": "Volume levelling",
  "clients.title": "Registered clients",
  "clients.detail":
    "Devices and scripts holding a credential on your library. Revoking one stops it at once.",
  "clients.name": "What it is for",
  "clients.register": "Register",
  "clients.once":
    "Copy this now. Only its hash is stored, so it is shown once and never again:",
  "clients.revoke": "Revoke",
  "clients.none": "No client holds a credential on your library.",
  "clients.error": "That did not work. Nothing changed.",
  "clients.registered": "registered {when}",
  "clients.used": "last used {when}",
  "clients.unused": "never used",
  "subsonic.title": "Subsonic password",
  "subsonic.detail":
    "The separate password Subsonic clients sign in with. Changing it replaces the API key too, so a leaked one stops working.",
  "subsonic.set": "set {when}",
  "subsonic.changed": "last changed {when}",
  "subsonic.password": "New password",
  "subsonic.rotate": "Change it",
  "subsonic.revoke": "Switch it off",
  "subsonic.confirm": "Yes, switch it off",
  "subsonic.once":
    "Copy this API key now. Only its hash is stored, so it is shown once and never again:",
  "subsonic.gone":
    "Your Subsonic clients will stop working until you set it again in each of them.",
  "subsonic.none":
    "You have no Subsonic password, so no Subsonic client can read your library. Ask the administrator for one.",
  "subsonic.short": "Use at least 12 characters.",
  "subsonic.error": "That did not work. Nothing changed.",
  "quality.label": "File quality",
  "quality.unknown": "Unknown",
  "quality.low": "Low",
  "quality.standard": "Standard",
  "quality.high": "High",
  "quality.lossless": "Lossless",
  "quality.hiRes": "Hi-res",
  "file.channels": { one: "mono", other: "{count} ch" },
  "common.delete": "Delete",
  "common.cancel": "Cancel",
} as const;
