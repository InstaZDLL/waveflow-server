// Words shared across screens, file sizes, card buttons and ratings.
import type { TranslationValue } from "..";
import type { common as en } from "../en/common";

export const common: Record<keyof typeof en, TranslationValue> = {
  "common.loading": "Caricamento",
  "common.loadError": "Non siamo riusciti a caricare questa vista",
  "common.noAlbums": "Ancora nessun album.",
  "common.noArtists": "Ancora nessun artista.",
  "common.variousArtists": "Artisti vari",
  "common.unknownArtist": "Artista sconosciuto",
  "common.album": "Album",
  "common.artist": "Artista",
  "common.albums": { one: "{count} album", other: "{count} album" },
  "common.tracks": { one: "{count} brano", other: "{count} brani" },
  "common.delete": "Elimina",
  "common.cancel": "Annulla",
  "common.back": "Indietro",
  "file.megabytes": "{size} MB",
  "file.kilobytes": "{size} kB",
  "file.channels": { one: "mono", other: "{count} canali" },
  "card.play": "Riproduci",
  "card.queue": "Aggiungi alla coda",
  "rating.label": "Valutazione",
  "rating.stars": { one: "{count} stella", other: "{count} stelle" },
  "source.offer": "WaveFlow Server {version} è software libero (AGPL-3.0).",
  "source.link": "Codice sorgente",
};
