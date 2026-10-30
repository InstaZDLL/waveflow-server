// Words shared across screens, file sizes, card buttons and ratings.
import type { TranslationValue } from "..";
import type { common as en } from "../en/common";

export const common: Record<keyof typeof en, TranslationValue> = {
  "common.loading": "Cargando",
  "common.loadError": "No pudimos cargar esta vista",
  "common.noAlbums": "Todavía no hay álbumes.",
  "common.noArtists": "Todavía no hay artistas.",
  "common.variousArtists": "Varios artistas",
  "common.unknownArtist": "Artista desconocido",
  "common.album": "Álbum",
  "common.artist": "Artista",
  "common.albums": { one: "{count} álbum", other: "{count} álbumes" },
  "common.tracks": { one: "{count} pista", other: "{count} pistas" },
  "common.delete": "Eliminar",
  "common.cancel": "Cancelar",
  "common.back": "Atrás",
  "file.megabytes": "{size} MB",
  "file.kilobytes": "{size} kB",
  "file.channels": { one: "mono", other: "{count} canales" },
  "card.play": "Reproducir",
  "card.queue": "Añadir a la cola",
  "rating.label": "Valoración",
  "rating.stars": { one: "{count} estrella", other: "{count} estrellas" },
  "source.offer": "WaveFlow Server {version} es software libre (AGPL-3.0).",
  "source.link": "Código fuente",
};
