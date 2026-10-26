// The attention lists and artist splits.
import type { TranslationValue } from "..";
import type { attention as en } from "../en/attention";

export const attention: Record<keyof typeof en, TranslationValue> = {
  "attention.lists": "Listes",
  "attention.total": {
    one: "{count} point à vérifier",
    other: "{count} points à vérifier",
  },
  "attention.nothing": "Rien à corriger.",
  "attention.notYours":
    "Seuls les propriétaires et les gestionnaires de cette bibliothèque voient ce qui y est à corriger.",
  "attention.empty": "Rien dans cette liste.",
  "attention.tracksNote":
    "Corrigez une piste depuis son menu : une fois corrigée, elle quitte la liste.",
  "attention.creditsNote":
    "Chacun de ces crédits nomme plusieurs artistes dans une seule chaîne. Scindez-le en ces artistes, ou gardez-le tel quel s’il s’agit d’un seul nom. Chaque scan suivant applique la décision.",
  "attention.pending": "À décider",
  "attention.decided": "Décidés",
  "attention.split": "Scinder",
  "attention.confirmSplit": "Confirmer : scinder en {count}",
  "attention.keep": "Garder tel quel",
  "attention.splitDone": "{name} est scindé.",
  "attention.keepDone": "{name} reste tel quel.",
  "attention.undone": "La décision sur {name} est annulée.",
  "attention.alreadyDecided":
    "Ce crédit a été décidé entre-temps. La liste a été rechargée.",
  "attention.failed": "Cela n’a pas fonctionné. Réessayez.",
  "attention.noCredits": "Aucun crédit n’attend de décision.",
  "attention.noDecisions": "Aucune décision pour l’instant.",
  "attention.fragments": "Ce en quoi {name} serait scindé",
  "attention.knownFragment": "Déjà un artiste de cette bibliothèque",
  "attention.decidedSplit": "Scindé en {parts}",
  "attention.decidedKeep": "Gardé tel quel",
  "attention.decidedBy": "Par {who}, le {when}",
  "attention.undo": "Annuler",
  "attention.duplicateGroup": "Doublon probable {group}",
  "attention.split_credits": "Artistes à scinder",
  "attention.missing_artist": "Sans artiste",
  "attention.missing_album": "Sans album",
  "attention.missing_year": "Sans année",
  "attention.missing_track_number": "Sans numéro de piste",
  "attention.missing_cover": "Sans pochette",
  "attention.album_year_conflict": "Albums à plusieurs années",
  "attention.compilation_no_album_artist": "Compilations sans artiste d’album",
  "attention.duplicate_track_number": "Numéros de piste en double",
  "attention.track_number_gap": "Trous dans la numérotation",
  "attention.probable_duplicate": "Doublons probables",
  "attention.probable_fake_lossless": "Faux lossless probables",
  "artistSplit.offer": "Scinder en {count} artistes",
  "artistSplit.confirm": "Confirmer : scinder en {count}",
  "artistSplit.done": "Scindé en {parts}.",
  "artistSplit.failed": "Ce crédit n’a pas pu être scindé. Réessayez.",
};
