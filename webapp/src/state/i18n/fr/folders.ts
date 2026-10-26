// Folders, filing and uploads.
import type { TranslationValue } from "..";
import type { folders as en } from "../en/folders";

export const folders: Record<keyof typeof en, TranslationValue> = {
  "folders.count": {
    one: "{count} dossier créé dans cette bibliothèque",
    other: "{count} dossiers créés dans cette bibliothèque",
  },
  "folders.where": "Chemin du dossier",
  "folders.subfolders": "Dossiers",
  "folders.noLibrary": "Aucune bibliothèque n’est accessible à ce compte.",
  "folders.none": "Aucun dossier n’a encore été créé dans cette bibliothèque.",
  "folders.system": "Tenu par WaveFlow",
  "folders.systemNote":
    "À ranger réunit toutes les pistes qui ne sont dans aucun dossier. WaveFlow le tient lui-même : il n’est pas compté parmi les dossiers ci-dessus, et il ne peut être ni renommé, ni déplacé, ni supprimé.",
  "filed.unfiled": "À ranger",
  "filed.automatic": "Automatique",
  "filed.topLevel": "Premier niveau",
  "filed.here": "Ici actuellement",
  "filed.destinations": "Dossiers au choix",
  "filed.loadError": "Les dossiers n’ont pas pu être chargés.",
  "filed.moveFailed": "Le déplacement a échoué. Réessayez.",
  "filed.moveTrack": "Déplacer « {title} » vers un dossier",
  "filed.moveFolder": "Déplacer « {name} »",
  "filed.note":
    "Des dossiers qui n’existent que dans WaveFlow. Classer une piste ici ne déplace aucun fichier.",
  "filed.newFolder": "Nouveau dossier",
  "filed.name": "Nom du dossier",
  "filed.renameLabel": "Nouveau nom",
  "filed.create": "Créer",
  "filed.save": "Enregistrer",
  "filed.cancel": "Annuler",
  "filed.rename": "Renommer",
  "filed.move": "Déplacer…",
  "filed.delete": "Supprimer",
  "filed.confirmDelete":
    "Supprimer « {name} » et ses sous-dossiers ? Leurs pistes reviennent dans À ranger ; aucun fichier n’est touché.",
  "filed.created": "« {name} » créé.",
  "filed.renamed": "Renommé en « {name} ».",
  "filed.deleted": {
    one: "Dossier supprimé : {count} piste revient dans À ranger.",
    other: "Dossier supprimé : {count} pistes reviennent dans À ranger.",
  },
  "filed.filed": {
    one: "{count} piste classée dans {folder}.",
    other: "{count} pistes classées dans {folder}.",
  },
  "filed.folderMoved": "« {name} » déplacé.",
  "filed.nameTaken": "Un dossier de ce nom existe déjà ici.",
  "filed.refused":
    "Ce dossier ne peut pas aller là : vérifiez son nom, ou l’arborescence est trop profonde ou pleine.",
  "filed.failed": "Cela n’a pas fonctionné. Réessayez.",
  "filed.missing": "Ce dossier n’existe plus.",
  "filed.empty": "Rien n’est encore classé ici.",
  "filed.allFiled": "Toutes les pistes sont classées.",
  "filed.uploadHere": "Envoyer ici",
  "filed.unfiledNote":
    "Toutes les pistes qui ne sont encore dans aucun dossier. WaveFlow tient cette liste lui-même : une piste en sort dès qu’elle est rangée, et elle ne peut être ni renommée ni supprimée. Pour ranger une piste, ouvrez son menu et choisissez « Déplacer vers un dossier… ».",
  "filed.unfiledNoteReader":
    "Toutes les pistes qui ne sont encore dans aucun dossier. WaveFlow tient cette liste lui-même : une piste en sort dès qu’elle est rangée.",
  "upload.into": "Les fichiers reçus vont dans le dossier {folder}.",
  "upload.doneUnfiled": "Ajouté à la bibliothèque, dans À ranger",
  "upload.folderMissing":
    "Le dossier pour lequel cet envoi a été ouvert est introuvable : les fichiers reçus iront dans À ranger.",
  "upload.folderUnchecked":
    "Le dossier pour lequel cet envoi a été ouvert n’a pas pu être vérifié : aucun fichier n’est pris pour l’instant.",
  "upload.checkAgain": "Vérifier à nouveau",
  "upload.title": "Téléverser dans {library}",
  "upload.detail":
    "Chaque fichier est d’abord identifié dans votre navigateur : un fichier que la bibliothèque possède déjà n’est jamais renvoyé, et un transfert interrompu reprend là où il s’était arrêté.",
  "upload.pick": "Choisir des fichiers audio",
  "upload.formats": "Formats que la bibliothèque sait indexer : {extensions}",
  "upload.noLibrary": "Choisissez d’abord une bibliothèque.",
  "upload.notAllowed":
    "Seul un propriétaire ou un gestionnaire de cette bibliothèque peut y ajouter des fichiers.",
  "upload.closed":
    "Cette bibliothèque n’accepte pas de fichiers. C’est son opérateur qui en décide, sur le serveur.",
  "upload.waiting": "En attente",
  "upload.hashing": "Identification…",
  "upload.negotiating": "Question au serveur…",
  "upload.uploading": "Envoi…",
  "upload.committing": "Ajout à la bibliothèque…",
  "upload.done": "Ajouté à la bibliothèque",
  "upload.present": "Déjà dans la bibliothèque",
  "upload.refused": "Refusé : {reason}",
  "upload.failed": "Échec : {reason}",
  "upload.progress": "Progression : {name}",
  "upload.reason.unsupportedFormat":
    "format audio que la bibliothèque ne sait pas indexer",
  "upload.reason.tooLarge": "plus gros que ce que le serveur accepte",
  "upload.reason.quotaExceeded": "la bibliothèque n’a plus de place",
  "upload.reason.libraryClosed": "la bibliothèque n’accepte pas de fichiers",
  "upload.reason.tooManySessions":
    "trop de transferts déjà ouverts — réessayez dans un instant",
  "upload.reason.refusedFile":
    "ce qui est arrivé ne correspond pas au fichier, ou n’est pas un audio que la bibliothèque sait lire",
  "upload.reason.unavailable":
    "le serveur n’a pas pu le prendre pour l’instant — réessayez",
  "upload.reason.network": "la connexion a été perdue",
  "upload.reason.notAllowed":
    "vous ne pouvez plus ajouter de fichiers à cette bibliothèque",
  "upload.reason.error": "quelque chose s’est mal passé",
  "upload.reason.hashing": "ce navigateur n’a pas pu identifier le fichier",
};
