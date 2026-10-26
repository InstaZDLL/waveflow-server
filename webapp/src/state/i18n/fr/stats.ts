// Listening statistics and mixes.
import type { TranslationValue } from "..";
import type { stats as en } from "../en/stats";

export const stats: Record<keyof typeof en, TranslationValue> = {
  "stats.detail": "Ce que ce compte a écouté, compté à partir de ses écoutes.",
  "stats.period": "Période",
  "stats.period7d": "7 derniers jours",
  "stats.period30d": "30 derniers jours",
  "stats.period365d": "12 derniers mois",
  "stats.periodAll": "Depuis toujours",
  "stats.empty": "Rien n’a été écouté pendant cette période.",
  "stats.plays": "Écoutes",
  "stats.listeningTime": "Temps d’écoute",
  "stats.tracks": "Titres",
  "stats.artists": "Artistes",
  "stats.albums": "Albums",
  "stats.estimate":
    "Le temps d’écoute additionne la durée de chaque titre écouté ; aucun lecteur ne dit quelle part en a été entendue.",
  "stats.timelineDays": "Écoutes par jour",
  "stats.timelineMonths": "Écoutes par mois",
  "stats.bucket": {
    one: "{label} : {count} écoute",
    other: "{label} : {count} écoutes",
  },
  "stats.topTracks": "Titres les plus écoutés",
  "stats.topArtists": "Artistes les plus écoutés",
  "stats.topAlbums": "Albums les plus écoutés",
  "stats.topGenres": "Genres les plus écoutés",
  "stats.playCount": { one: "{count} écoute", other: "{count} écoutes" },
  "stats.firstPlay": "Première écoute de la période",
  "stats.lastPlay": "Dernière écoute",
  "stats.moment": "{title}, {when}",
  "stats.minutes": { one: "{count} minute", other: "{count} minutes" },
  "stats.hoursMinutes": "{hours} h {minutes} min",
  "mixes.title": "Pour vous aujourd’hui",
  "mixes.detail": {
    one: "{count} titre, tiré pour aujourd’hui",
    other: "{count} titres, tirés pour aujourd’hui",
  },
  "mixes.mostPlayed": "Les plus écoutés",
  "mixes.neverPlayed": "Jamais écoutés",
  "mixes.forgottenFavourites": "Favoris oubliés",
  "mixes.recentlyAdded": "Ajouts récents",
  "mixes.genre": "Mix {genre}",
  "mixes.decade": "Les années {decade}",
  "mixes.failed": "Impossible d’ouvrir « {title} ». Réessayez.",
  "mixes.shelfTitle": "Mix pour vous",
  "mixes.shelfDetail":
    "Des listes tirées de vos écoutes, renouvelées chaque jour. Vos albums sont en dessous.",
  "mixes.badge": "Mix",
};
