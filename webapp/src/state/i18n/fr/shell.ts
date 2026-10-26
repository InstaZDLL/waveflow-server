// Navigation, preferences, ReplayGain, shared words, album and song views.
import type { TranslationValue } from "..";
import type { shell as en } from "../en/shell";

export const shell: Record<keyof typeof en, TranslationValue> = {
  "nav.albums": "Albums",
  "nav.artists": "Artistes",
  "nav.songs": "Titres",
  "nav.search": "Recherche",
  "nav.favourites": "Favoris",
  "nav.playlists": "Playlists",
  "nav.queue": "File d’attente",
  "nav.shares": "Partages",
  "nav.offline": "Hors ligne",
  "nav.radio": "Radio",
  "nav.scrobbling": "Scrobbling",
  "nav.admin": "Administration",
  "nav.signOut": "Se déconnecter",
  "nav.skip": "Aller au contenu",
  "nav.home": "WaveFlow serveur, accueil",
  "nav.server": "serveur",
  "nav.more": "Plus",
  "nav.moreTitle": "Tout le reste",
  "nav.moreDetail":
    "La barre du bas porte ce qu’on attrape en écoutant. Le reste est ici.",
  "nav.moreClose": "Fermer",
  "preferences.theme": "Thème",
  "preferences.language": "Langue",
  "preferences.themeLight": "Clair",
  "preferences.themeDark": "Sombre",
  "preferences.followSystem": "Suivre le système",
  "preferences.followSystemDetail":
    "Passer de la version claire d’un thème à sa version sombre en même temps que votre appareil.",
  "preferences.followingLight":
    "Votre appareil est en mode clair : la version claire est affichée. Choisir un thème choisit ses deux versions.",
  "preferences.followingDark":
    "Votre appareil est en mode sombre : la version sombre est affichée. Choisir un thème choisit ses deux versions.",
  "preferences.standInLight":
    "{theme} n’a pas de version claire : {fallback} le remplace tant que votre appareil est en mode clair.",
  "preferences.standInDark":
    "{theme} n’a pas de version sombre : {fallback} le remplace tant que votre appareil est en mode sombre.",
  "replayGain.label": "ReplayGain",
  "replayGain.off": "Désactivé",
  "replayGain.track": "Gain par piste",
  "replayGain.album": "Gain par album",
  "replayGain.preamp": "Préampli",
  "replayGain.detail":
    "Joue chaque piste au volume mesuré dans ses tags, pour qu’un disque ne sonne pas plus fort que le suivant. Le gain par album garde les écarts à l’intérieur d’un album. Une piste sans tags est jouée telle quelle. La lecture passe alors par Web Audio, que certains téléphones mettent en pause avec l’écran.",
  "common.loading": "Chargement",
  "common.loadError": "Impossible de charger cette vue",
  "common.noAlbums": "Aucun album pour le moment.",
  "common.noArtists": "Aucun artiste pour le moment.",
  "common.variousArtists": "Artistes variés",
  "common.unknownArtist": "Artiste inconnu",
  "common.album": "Album",
  "album.play": "Lire l’album",
  "album.addQueue": "Ajouter à la file",
  "album.title": "Titre",
  "album.duration": "Durée",
  "album.trackActions": "Actions sur les titres",
  "album.moreBy": "Autres albums de {artist}",
  "common.artist": "Artiste",
  "common.albums": { one: "{count} album", other: "{count} albums" },
  "common.tracks": { one: "{count} piste", other: "{count} pistes" },
  "songs.loadedSoFar": {
    one: "{count} piste chargée",
    other: "{count} pistes chargées",
  },
  "songs.playLoaded": {
    one: "Lire la piste chargée",
    other: "Lire les {count} pistes chargées",
  },
  "songs.playAll": {
    one: "Lire la piste",
    other: "Lire les {count} pistes",
  },
  "songs.loadMore": "Charger la suite",
  "songs.loadMoreError": "Cette page n’a pas pu être chargée. Réessayez.",
  "songs.loading": "Chargement…",
  "songs.empty": "Cette bibliothèque ne contient encore aucune piste.",
  "file.megabytes": "{size} Mo",
  "file.kilobytes": "{size} ko",
};
