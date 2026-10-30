// Words shared across screens, file sizes, card buttons and ratings.
import type { TranslationValue } from "..";
import type { common as en } from "../en/common";

export const common: Record<keyof typeof en, TranslationValue> = {
  "common.loading": "Wird geladen",
  "common.loadError": "Diese Ansicht konnte nicht geladen werden",
  "common.noAlbums": "Noch keine Alben.",
  "common.noArtists": "Noch keine Künstler.",
  "common.variousArtists": "Verschiedene Künstler",
  "common.unknownArtist": "Unbekannter Künstler",
  "common.album": "Album",
  "common.artist": "Künstler",
  "common.albums": { one: "{count} Album", other: "{count} Alben" },
  "common.tracks": { one: "{count} Titel", other: "{count} Titel" },
  "common.delete": "Löschen",
  "common.cancel": "Abbrechen",
  "common.back": "Zurück",
  "file.megabytes": "{size} MB",
  "file.kilobytes": "{size} kB",
  "file.channels": { one: "Mono", other: "{count} Kanäle" },
  "card.play": "Abspielen",
  "card.queue": "Zur Warteschlange hinzufügen",
  "rating.label": "Bewertung",
  "rating.stars": { one: "{count} Stern", other: "{count} Sterne" },
  "source.offer": "WaveFlow Server {version} ist freie Software (AGPL-3.0).",
  "source.link": "Quellcode",
};
