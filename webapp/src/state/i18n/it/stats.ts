// Listening statistics and mixes.
import type { TranslationValue } from "..";
import type { stats as en } from "../en/stats";

export const stats: Record<keyof typeof en, TranslationValue> = {
  "stats.detail":
    "Cosa ha ascoltato questo account, calcolato dai suoi ascolti.",
  "stats.period": "Periodo",
  "stats.period7d": "Ultimi 7 giorni",
  "stats.period30d": "Ultimi 30 giorni",
  "stats.period365d": "Ultimi 12 mesi",
  "stats.periodAll": "Sempre",
  "stats.empty": "Nessun ascolto in questo periodo.",
  "stats.plays": "Ascolti",
  "stats.listeningTime": "Tempo di ascolto",
  "stats.tracks": "Brani",
  "stats.artists": "Artisti",
  "stats.albums": "Album",
  "stats.estimate":
    "Il tempo di ascolto somma la durata di ogni brano ascoltato; nessun lettore dice quanta parte ne sia stata effettivamente ascoltata.",
  "stats.timelineDays": "Ascolti al giorno",
  "stats.timelineMonths": "Ascolti al mese",
  "stats.bucket": {
    one: "{label}: {count} ascolto",
    other: "{label}: {count} ascolti",
  },
  "stats.topTracks": "Brani più ascoltati",
  "stats.topArtists": "Artisti più ascoltati",
  "stats.topAlbums": "Album più ascoltati",
  "stats.topGenres": "Generi più ascoltati",
  "stats.playCount": { one: "{count} ascolto", other: "{count} ascolti" },
  "stats.firstPlay": "Primo ascolto del periodo",
  "stats.lastPlay": "Ultimo ascolto",
  "stats.moment": "{title}, {when}",
  "stats.minutes": { one: "{count} minuto", other: "{count} minuti" },
  "stats.hoursMinutes": "{hours} h {minutes} min",
  "mixes.title": "Creati per te oggi",
  "mixes.detail": {
    one: "{count} brano, scelto per oggi",
    other: "{count} brani, scelti per oggi",
  },
  "mixes.mostPlayed": "I più ascoltati",
  "mixes.neverPlayed": "Mai ascoltati",
  "mixes.forgottenFavourites": "Preferiti dimenticati",
  "mixes.recentlyAdded": "Aggiunti di recente",
  "mixes.genre": "Mix {genre}",
  "mixes.decade": "Anni {decade}",
  "mixes.failed": "Impossibile aprire «{title}». Riprova.",
  "mixes.shelfTitle": "Mix per te",
  "mixes.shelfDetail":
    "Playlist create a partire dai tuoi ascolti, nuove ogni giorno. I tuoi album sono qui sotto.",
  "mixes.badge": "Mix",
};
