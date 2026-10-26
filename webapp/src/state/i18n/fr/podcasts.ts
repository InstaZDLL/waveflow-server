// Podcast subscriptions and episodes.
import type { TranslationValue } from "..";
import type { podcasts as en } from "../en/podcasts";

export const podcasts: Record<keyof typeof en, TranslationValue> = {
  "podcasts.detail": "Les émissions que suit ce serveur, pour tous les comptes",
  "podcasts.off":
    "Les podcasts sont désactivés sur ce serveur. Ce qui a été conservé reste lisible ; rien de nouveau n’est récupéré.",
  "podcasts.url": "Adresse du flux",
  "podcasts.shared":
    "Visible par tous les comptes : chacun sur ce serveur voit l’émission et peut écouter les épisodes téléchargés. Seuls les administrateurs voient l’adresse du flux.",
  "podcasts.subscribe": "S’abonner",
  "podcasts.refresh": "Actualiser tous les flux",
  "podcasts.refreshing": "Les flux sont en cours de récupération.",
  "podcasts.reload": "Recharger",
  "podcasts.empty": "Aucun podcast pour l’instant.",
  "podcasts.emptyDetail":
    "Un administrateur abonne le serveur au flux d’une émission ici. Le serveur télécharge ses épisodes, et tous les comptes de ce serveur peuvent les écouter.",
  "podcasts.addFirst": "Ajouter votre premier podcast",
  "podcasts.channels": "Émissions",
  "podcasts.showing": "Épisodes de {title} affichés.",
  "podcasts.noEpisodes": "Ce flux ne contient aucun épisode.",
  "podcasts.episodes": {
    one: "{count} épisode",
    other: "{count} épisodes",
  },
  "podcasts.failed": "dernière récupération en échec : {reason}",
  "podcasts.open": "Épisodes",
  "podcasts.hide": "Masquer",
  "podcasts.unsubscribe": "Se désabonner",
  "podcasts.download": "Télécharger",
  "podcasts.play": "Lire {title}",
  "podcasts.pause": "Mettre en pause {title}",
  "podcasts.resumeAt": "reprise à {time}",
  "podcasts.status.new": "non téléchargé",
  "podcasts.status.downloading": "téléchargement en cours",
  "podcasts.status.completed": "prêt",
  "podcasts.status.error": "échec du téléchargement",
  "podcasts.status.deleted": "supprimé",
  "podcasts.conflict": "Ce serveur suit déjà ce flux.",
  "podcasts.refused":
    "Le flux n’a pas pu être récupéré ou lu, ou son adresse n’est pas joignable par ce serveur.",
  "podcasts.subscribeError": "Le flux n’a pas pu être ajouté.",
  "podcasts.mutationError": "Cela n’a pas fonctionné. Réessayez.",
  "podcasts.playbackError": "Cet épisode n’a pas pu être lu.",
};
