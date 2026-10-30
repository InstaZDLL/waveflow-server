// Listening statistics and mixes.
import type { TranslationValue } from "..";
import type { stats as en } from "../en/stats";

export const stats: Record<keyof typeof en, TranslationValue> = {
  "stats.detail":
    "Lo que ha escuchado esta cuenta, contado a partir de sus propias reproducciones.",
  "stats.period": "Período",
  "stats.period7d": "Últimos 7 días",
  "stats.period30d": "Últimos 30 días",
  "stats.period365d": "Últimos 12 meses",
  "stats.periodAll": "Desde siempre",
  "stats.empty": "No se escuchó nada en este período.",
  "stats.plays": "Escuchas",
  "stats.listeningTime": "Tiempo de escucha",
  "stats.tracks": "Pistas",
  "stats.artists": "Artistas",
  "stats.albums": "Álbumes",
  "stats.estimate":
    "El tiempo de escucha suma la duración de cada pista escuchada; ningún reproductor indica cuánto se oyó de ella.",
  "stats.timelineDays": "Escuchas por día",
  "stats.timelineMonths": "Escuchas por mes",
  "stats.bucket": {
    one: "{label}: {count} escucha",
    other: "{label}: {count} escuchas",
  },
  "stats.topTracks": "Pistas más escuchadas",
  "stats.topArtists": "Artistas más escuchados",
  "stats.topAlbums": "Álbumes más escuchados",
  "stats.topGenres": "Géneros más escuchados",
  "stats.playCount": { one: "{count} escucha", other: "{count} escuchas" },
  "stats.firstPlay": "Primera escucha del período",
  "stats.lastPlay": "Última escucha",
  "stats.moment": "{title}, {when}",
  "stats.minutes": { one: "{count} minuto", other: "{count} minutos" },
  "stats.hoursMinutes": "{hours} h {minutes} min",
  "mixes.title": "Hecho para ti hoy",
  "mixes.detail": {
    one: "{count} pista, elegida para hoy",
    other: "{count} pistas, elegidas para hoy",
  },
  "mixes.mostPlayed": "Lo más escuchado",
  "mixes.neverPlayed": "Nunca escuchado",
  "mixes.forgottenFavourites": "Favoritos olvidados",
  "mixes.recentlyAdded": "Añadido recientemente",
  "mixes.genre": "Mezcla de {genre}",
  "mixes.decade": "Década de {decade}",
  "mixes.failed": "No se pudo abrir “{title}”. Vuelve a intentarlo.",
  "mixes.shelfTitle": "Mezclas para ti",
  "mixes.shelfDetail":
    "Listas sacadas de lo que escuchas, nuevas cada día. Tus álbumes están más abajo.",
  "mixes.badge": "Mezcla",
};
