// Administration, scans, maintenance and missing files.
import type { TranslationValue } from "..";
import type { admin as en } from "../en/admin";

export const admin: Record<keyof typeof en, TranslationValue> = {
  "admin.title": "Administration",
  "admin.detail": "Bibliothèques, scans et comptes",
  "admin.libraries": "Bibliothèques",
  "admin.libraryName": "Nom de la bibliothèque",
  "admin.libraryPath": "Chemin absolu du dossier sur le serveur",
  "admin.registering": "Ajout en cours…",
  "admin.register": "Ajouter et scanner",
  "admin.scan": "Scanner",
  "admin.accounts": "Comptes",
  "admin.webPassword": "Mot de passe web, 12 caractères minimum",
  "admin.creating": "Création…",
  "admin.create": "Créer le compte",
  "admin.enable": "Activer",
  "admin.disable": "Désactiver",
  "admin.currentAccount": "Vous ne pouvez pas désactiver votre compte actuel",
  "admin.libraryError": "Le dossier n’a pas pu être enregistré.",
  "admin.accountError": "Le compte n’a pas pu être créé.",
  "admin.statusError": "Le statut du compte n’a pas pu être modifié.",
  "admin.scanError": "Le scan n’a pas pu démarrer.",
  "admin.scanQueued": "Scan {id} ajouté à la file.",
  "admin.initialScan": "Scan initial {id} ajouté à la file.",
  "admin.scanning": "Analyse en cours",
  "admin.nowPlaying": "Écoutes en cours",
  "admin.nobodyListening": "Personne n’écoute en ce moment.",
  "admin.tokens": "Jetons d’API de {username}",
  "admin.tokenName": "À quoi sert-il ?",
  "admin.tokenCreate": "Émettre",
  "admin.tokenOnce": "Copiez-le maintenant. Il n’est montré qu’une fois :",
  "admin.tokenError": "Le jeton n’a pas pu être émis.",
  "admin.tokenRevoke": "Révoquer",
  "admin.tokenUsed": "déjà utilisé",
  "admin.tokenUnused": "jamais utilisé",
  "admin.tokenNone": "Aucun jeton sur ce compte.",
  "admin.members": "Qui peut voir cette bibliothèque",
  "admin.memberRole": "Rôle",
  "admin.memberRemove": "Retirer",
  "admin.memberOwner": "propriétaire, et le reste",
  "admin.memberAdd": "Donner accès à",
  "admin.memberChoose": "Choisir un compte…",
  "admin.memberError": "Cette appartenance n’a pas pu être modifiée.",
  "admin.manage": "Nom, dossier et suppression",
  "admin.rename": "Nom",
  "admin.path": "Dossier sur le serveur",
  "admin.followLinks":
    "Suivre les liens symboliques dont la cible reste dans la bibliothèque",
  "admin.watch":
    "Surveiller le dossier et analyser ce qui change (à couper pour un partage réseau modifié par d’autres machines)",
  "admin.save": "Enregistrer",
  "admin.saved": "Enregistré.",
  "admin.savedScan": "Enregistré. Scan {id} ajouté à la file.",
  "admin.saveError":
    "Ce changement n’a pas pu être enregistré. Le dossier doit exister sur le serveur et ne pas être un lien symbolique.",
  "admin.saveBusy":
    "La bibliothèque est en cours d’analyse, ou une autre bibliothèque utilise déjà ce dossier. Réessayez une fois l’analyse terminée.",
  "admin.delete": "Supprimer cette bibliothèque",
  "admin.deleteNote":
    "Retire la bibliothèque du catalogue pour tous ses membres : ses titres, albums et artistes, avec leurs entrées de playlist, favoris et écoutes. Aucun fichier n’est touché sur le disque. Tapez le nom de la bibliothèque pour confirmer.",
  "admin.deleteConfirm": "Le nom de la bibliothèque, pour confirmer",
  "admin.deleteButton": "Supprimer définitivement",
  "admin.deleted":
    "{name} a été supprimée : {tracks} titres ont quitté le catalogue.",
  "admin.deleteError": "La bibliothèque n’a pas pu être supprimée.",
  "scan.connecting": "En attente du premier relevé…",
  "scan.queued": "En attente",
  "scan.running": "Analyse en cours",
  "scan.completed": "Terminé",
  "scan.failed": "Échec",
  "scan.progress": "Progression de l’analyse",
  "scan.counted": "{processed} fichiers sur {total}",
  "scan.tally":
    "{added} ajoutés · {updated} mis à jour · {moved} déplacés · {skipped} inchangés · {errors} erreurs",
  "scan.lost":
    "Le flux de progression n’a pas pu s’ouvrir. L’analyse elle-même se poursuit.",
  "scan.leftOut": "{excluded} écartés · {links} liens non suivis",
  "maintenance.sections": "Sections de maintenance",
  "maintenance.none":
    "Rien ici n’est à vous d’entretenir dans cette bibliothèque.",
  "missing.total": {
    one: "{count} fichier que les scans ne trouvent plus",
    other: "{count} fichiers que les scans ne trouvent plus",
  },
  "missing.nothing": "Chaque fichier est là où le catalogue l’attend.",
  "missing.notYours":
    "Seuls les propriétaires et gestionnaires de cette bibliothèque, et les administrateurs, voient ses fichiers manquants.",
  "missing.note":
    "Ces titres restent au catalogue, avec leurs favoris, entrées de playlist et écoutes, jusqu’à leur purge ; un fichier qui revient avant garde son titre. Une purge peut être annulée pendant un temps, puis le titre, et tout ce qui s’y rattache pour chaque membre, disparaît pour de bon.",
  "missing.since": "manquant depuis le {date}",
  "missing.cost":
    "{playlists} en playlist · {favorites} favoris · {plays} écoutes",
  "missing.select": "Sélectionner {title}",
  "missing.purgeSelected": "Purger la sélection ({count})",
  "missing.purgeAll": "Tout purger",
  "missing.confirm": "Confirmer : purger {count}",
  "missing.purged": {
    one: "{count} titre purgé.",
    other: "{count} titres purgés.",
  },
  "missing.scanning":
    "La bibliothèque est en cours d’analyse. Réessayez une fois l’analyse terminée.",
  "missing.failed": "La purge n’a pas abouti. Réessayez.",
  "missing.more": "Afficher plus",
  "missing.purgedUntil": {
    one: "{count} titre purgé. Il peut être restauré jusqu’au {date}.",
    other: "{count} titres purgés. Ils peuvent être restaurés jusqu’au {date}.",
  },
  "missing.purgedTitle": {
    one: "Purgés, encore restaurables ({count})",
    other: "Purgés, encore restaurables ({count})",
  },
  "missing.purgedNote":
    "Rien n’en est encore perdu. Restauré, un titre retourne aux fichiers manquants avec tout ce qui s’y rattachait ; un fichier qui revient restaure son titre de lui-même.",
  "missing.deletedAfter": "supprimé pour de bon après le {date}",
  "missing.restore": "Restaurer",
  "missing.restoreOne": "Restaurer {title}",
  "missing.restoreAll": "Tout restaurer",
  "missing.restored": {
    one: "{count} titre restauré.",
    other: "{count} titres restaurés.",
  },
  "missing.restoreFailed": "La restauration n’a pas abouti. Réessayez.",
};
