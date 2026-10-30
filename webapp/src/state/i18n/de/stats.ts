// Listening statistics and mixes.
import type { TranslationValue } from "..";
import type { stats as en } from "../en/stats";

export const stats: Record<keyof typeof en, TranslationValue> = {
  "stats.detail":
    "Was dieses Konto gehört hat, gezählt aus seinen eigenen Wiedergaben.",
  "stats.period": "Zeitraum",
  "stats.period7d": "Letzte 7 Tage",
  "stats.period30d": "Letzte 30 Tage",
  "stats.period365d": "Letzte 12 Monate",
  "stats.periodAll": "Gesamte Zeit",
  "stats.empty": "In diesem Zeitraum wurde nichts gehört.",
  "stats.plays": "Wiedergaben",
  "stats.listeningTime": "Hörzeit",
  "stats.tracks": "Titel",
  "stats.artists": "Künstler",
  "stats.albums": "Alben",
  "stats.estimate":
    "Die Hörzeit addiert die Länge jedes gehörten Titels; kein Player meldet, wie viel davon tatsächlich gehört wurde.",
  "stats.timelineDays": "Wiedergaben pro Tag",
  "stats.timelineMonths": "Wiedergaben pro Monat",
  "stats.bucket": {
    one: "{label}: {count} Wiedergabe",
    other: "{label}: {count} Wiedergaben",
  },
  "stats.topTracks": "Top-Titel",
  "stats.topArtists": "Top-Künstler",
  "stats.topAlbums": "Top-Alben",
  "stats.topGenres": "Top-Genres",
  "stats.playCount": {
    one: "{count} Wiedergabe",
    other: "{count} Wiedergaben",
  },
  "stats.firstPlay": "Erste Wiedergabe im Zeitraum",
  "stats.lastPlay": "Letzte Wiedergabe",
  "stats.moment": "{title}, {when}",
  "stats.minutes": { one: "{count} Minute", other: "{count} Minuten" },
  "stats.hoursMinutes": "{hours} Std. {minutes} Min.",
  "mixes.title": "Heute für dich",
  "mixes.detail": {
    one: "{count} Titel, für heute ausgewählt",
    other: "{count} Titel, für heute ausgewählt",
  },
  "mixes.mostPlayed": "Meistgehört",
  "mixes.neverPlayed": "Nie gehört",
  "mixes.forgottenFavourites": "Vergessene Favoriten",
  "mixes.recentlyAdded": "Zuletzt hinzugefügt",
  "mixes.genre": "{genre}-Mix",
  "mixes.decade": "Die {decade}er",
  "mixes.failed":
    "„{title}“ konnte nicht geöffnet werden. Versuch es noch einmal.",
  "mixes.shelfTitle": "Mixe für dich",
  "mixes.shelfDetail":
    "Playlists aus deinem Hörverhalten, jeden Tag neu. Deine Alben findest du darunter.",
  "mixes.badge": "Mix",
};
