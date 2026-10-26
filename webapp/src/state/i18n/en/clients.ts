// Registered clients, the Subsonic password and OAuth consent.
export const clients = {
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
  "credential.password": "Dedicated Subsonic password",
  "credential.rotate": "Rotate credential",
  "credential.copy": "Copy this API key now:",
  "credential.error": "Credential rotation failed.",
  "authorize.title": "Authorisation",
  "authorize.invalid":
    "This authorisation link is incomplete or asks to send you somewhere we do not trust.",
  "authorize.client": "Authorise {client}",
  "authorize.detail":
    "It will be able to browse your libraries, play your music and manage your playlists, favourites and ratings, everything this account can do.",
  "authorize.redirect": "Sending you back to {url}",
  "authorize.error":
    "The application sent an authorisation request we cannot honour.",
  "authorize.busy": "Authorising…",
  "authorize.approve": "Authorise",
} as const;
