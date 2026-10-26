// Registered clients, the Subsonic password and OAuth consent.
import type { TranslationValue } from "..";
import type { clients as en } from "../en/clients";

export const clients: Record<keyof typeof en, TranslationValue> = {
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
  "credential.password": "Mot de passe Subsonic dédié",
  "credential.rotate": "Renouveler l’identifiant",
  "credential.copy": "Copiez cette clé API maintenant :",
  "credential.error": "Le renouvellement de l’identifiant a échoué.",
  "authorize.title": "Autorisation",
  "authorize.invalid":
    "Ce lien d’autorisation est incomplet ou mène vers une destination non fiable.",
  "authorize.client": "Autoriser {client}",
  "authorize.detail":
    "Cette application pourra parcourir vos bibliothèques, lire votre musique et gérer vos playlists, favoris et notes, comme ce compte.",
  "authorize.redirect": "Retour vers {url}",
  "authorize.error": "La demande d’autorisation de l’application est invalide.",
  "authorize.busy": "Autorisation…",
  "authorize.approve": "Autoriser",
};
