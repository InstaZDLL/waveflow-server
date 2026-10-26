// Settings, registered clients, the Subsonic password and quality labels.
import type { TranslationValue } from "..";
import type { settings as en } from "../en/settings";

export const settings: Record<keyof typeof en, TranslationValue> = {
  "nav.settings": "Réglages",
  "settings.detail":
    "Apparence, langue, et les clients autorisés à lire votre bibliothèque.",
  "settings.appearance": "Apparence",
  "settings.playback": "Lecture",
  "settings.sections": "Sections des réglages",
  "settings.section.account": "Compte",
  "settings.playbackDetail":
    "Comment ce navigateur lit vos titres. Gardé dans ce navigateur, pas sur votre compte.",
  "settings.levelling": "Normalisation du volume",
  "clients.title": "Clients enregistrés",
  "clients.detail":
    "Appareils et scripts détenant un justificatif sur votre bibliothèque. En révoquer un l’arrête aussitôt.",
  "clients.name": "À quoi il sert",
  "clients.register": "Enregistrer",
  "clients.once":
    "Copiez-le maintenant. Seule son empreinte est conservée : il est montré une fois et jamais plus :",
  "clients.revoke": "Révoquer",
  "clients.none":
    "Aucun client ne détient de justificatif sur votre bibliothèque.",
  "clients.error": "Cela n’a pas fonctionné. Rien n’a changé.",
  "clients.registered": "enregistré le {when}",
  "clients.used": "dernier usage le {when}",
  "clients.unused": "jamais utilisé",
  "subsonic.title": "Mot de passe Subsonic",
  "subsonic.detail":
    "Le mot de passe distinct avec lequel les clients Subsonic se connectent. Le changer remplace aussi la clé d’API, donc une clé divulguée cesse de fonctionner.",
  "subsonic.set": "défini le {when}",
  "subsonic.changed": "dernier changement le {when}",
  "subsonic.password": "Nouveau mot de passe",
  "subsonic.rotate": "Le changer",
  "subsonic.revoke": "Le désactiver",
  "subsonic.confirm": "Oui, le désactiver",
  "subsonic.once":
    "Copiez cette clé d’API maintenant. Seule son empreinte est conservée : elle est montrée une fois et jamais plus :",
  "subsonic.gone":
    "Vos clients Subsonic cesseront de fonctionner jusqu’à ce que vous le redéfinissiez dans chacun d’eux.",
  "subsonic.none":
    "Vous n’avez pas de mot de passe Subsonic, donc aucun client Subsonic ne peut lire votre bibliothèque. Demandez-en un à l’administrateur.",
  "subsonic.short": "Utilisez au moins 12 caractères.",
  "subsonic.error": "Cela n’a pas fonctionné. Rien n’a changé.",
  "quality.label": "Qualité du fichier",
  "quality.unknown": "Inconnue",
  "quality.low": "Faible",
  "quality.standard": "Standard",
  "quality.high": "Élevée",
  "quality.lossless": "Sans perte",
  "quality.hiRes": "Haute résolution",
  "file.channels": { one: "mono", other: "{count} canaux" },
  "common.delete": "Supprimer",
  "common.cancel": "Annuler",
};
