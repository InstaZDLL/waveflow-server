// Podcast subscriptions and episodes.
import type { TranslationValue } from "..";
import type { podcasts as en } from "../en/podcasts";

export const podcasts: Record<keyof typeof en, TranslationValue> = {
  "podcasts.detail": "Sendungen, denen dieser Server für alle Konten folgt",
  "podcasts.off":
    "Podcasts sind auf diesem Server abgeschaltet. Was bereits gespeichert wurde, bleibt abrufbar; Neues wird nicht geladen.",
  "podcasts.url": "Feed-Adresse",
  "podcasts.shared":
    "Für alle Konten sichtbar: Jeder auf diesem Server sieht die Sendung und kann die heruntergeladenen Folgen abspielen. Nur Administratoren sehen die Feed-Adresse.",
  "podcasts.subscribe": "Abonnieren",
  "podcasts.refresh": "Alle Feeds aktualisieren",
  "podcasts.refreshing": "Die Feeds werden neu abgerufen.",
  "podcasts.reload": "Neu laden",
  "podcasts.empty": "Noch keine Podcasts.",
  "podcasts.emptyDetail":
    "Ein Administrator abonniert hier den Feed einer Sendung. Der Server lädt ihre Folgen herunter, und alle Konten auf diesem Server können sie hören.",
  "podcasts.addFirst": "Ersten Podcast hinzufügen",
  "podcasts.channels": "Sendungen",
  "podcasts.showing": "Folgen von {title} werden angezeigt.",
  "podcasts.noEpisodes": "Dieser Feed hat keine Folgen.",
  "podcasts.episodes": {
    one: "{count} Folge",
    other: "{count} Folgen",
  },
  "podcasts.failed": "letzter Abruf fehlgeschlagen: {reason}",
  "podcasts.open": "Folgen",
  "podcasts.hide": "Ausblenden",
  "podcasts.unsubscribe": "Abo beenden",
  "podcasts.download": "Herunterladen",
  "podcasts.play": "{title} abspielen",
  "podcasts.pause": "{title} pausieren",
  "podcasts.resumeAt": "weiter bei {time}",
  "podcasts.status.new": "nicht heruntergeladen",
  "podcasts.status.downloading": "wird heruntergeladen",
  "podcasts.status.completed": "bereit",
  "podcasts.status.error": "Download fehlgeschlagen",
  "podcasts.status.deleted": "gelöscht",
  "podcasts.conflict": "Dieser Server folgt diesem Feed bereits.",
  "podcasts.refused":
    "Der Feed konnte nicht abgerufen oder gelesen werden, oder seine Adresse ist für diesen Server nicht erreichbar.",
  "podcasts.subscribeError": "Der Feed konnte nicht hinzugefügt werden.",
  "podcasts.mutationError": "Das hat nicht geklappt. Versuch es noch einmal.",
  "podcasts.playbackError": "Diese Folge konnte nicht abgespielt werden.",
};
