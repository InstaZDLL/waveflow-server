// Words shared across screens, file sizes, card buttons and ratings.
import type { TranslationValue } from "..";
import type { common as en } from "../en/common";

export const common: Record<keyof typeof en, TranslationValue> = {
  "common.loading": "Chargement",
  "common.loadError": "Impossible de charger cette vue",
  "common.noAlbums": "Aucun album pour le moment.",
  "common.noArtists": "Aucun artiste pour le moment.",
  "common.variousArtists": "Artistes variés",
  "common.unknownArtist": "Artiste inconnu",
  "common.album": "Album",
  "common.artist": "Artiste",
  "common.albums": { one: "{count} album", other: "{count} albums" },
  "common.tracks": { one: "{count} piste", other: "{count} pistes" },
  "common.delete": "Supprimer",
  "common.cancel": "Annuler",
  "common.back": "Retour",
  "file.megabytes": "{size} Mo",
  "file.kilobytes": "{size} ko",
  "file.channels": { one: "mono", other: "{count} canaux" },
  "card.play": "Lire",
  "card.queue": "Mettre en file",
  "rating.label": "Note",
  "rating.stars": { one: "{count} étoile", other: "{count} étoiles" },
};
