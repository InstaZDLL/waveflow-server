// The attention lists and the audio quality analysis.
import type { TranslationValue } from "..";
import type { quality as en } from "../en/quality";

export const quality: Record<keyof typeof en, TranslationValue> = {
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
  "quality.help":
    "Mesuré dans le son plutôt que lu dans le conteneur : où s’arrêtent les aigus, et combien de bits les échantillons utilisent. Un fichier sans perte dont les aigus s’arrêtent vers 16 kHz était un MP3 à 128 kb/s avant d’être un FLAC. « Aucun signe de source avec perte » n’est pas une preuve : un encodeur qui garde toute la bande ne laisse rien à trouver.",
  "quality.progress": "{measured} fichiers sans perte analysés sur {lossless}",
  "quality.disabled": "L’analyse est désactivée sur ce serveur.",
  "quality.noLossless": "Aucun fichier sans perte dans cette bibliothèque.",
  "quality.status.running":
    "Analyse en cours : {measured} sur {lossless} terminés, {inFlight} en traitement.",
  "quality.status.waitingForScan":
    "En attente de la fin du scan avant d’analyser.",
  "quality.status.queued":
    "En file d’attente : l’analyse arrivera bientôt à ces fichiers.",
  "quality.status.idle": "Au repos. Dernier fichier analysé le {when}.",
  "quality.status.idleNever": "Au repos. Aucun fichier n’a encore été analysé.",
  "quality.filters": "Verdicts",
  "quality.all": "Tous les fichiers",
  "quality.format": "Format",
  "quality.allFormats": "Tous les formats",
  "quality.empty": "Aucun fichier avec ce verdict.",
  "quality.column.track": "Morceau",
  "quality.column.container": "Ce que dit le fichier",
  "quality.column.highs": "Les aigus s’arrêtent à",
  "quality.column.verdict": "Ce que dit le son",
  "quality.fullBand": "Bande complète",
  "quality.khz": "{value} kHz",
  "quality.db": "{value} dB",
  "quality.analyseAgain": "Analyser à nouveau",
  "quality.confirmAnalyse": "Confirmer : réanalyser toute la bibliothèque",
  "quality.requested": "La bibliothèque sera réanalysée en arrière-plan.",
  "quality.requestFailed": "Ça n’a pas marché. Réessayez.",
  "quality.section": "Qualité réelle",
  "quality.verdictLabel": "Verdict",
  "quality.highsStop": "Les aigus s’arrêtent à",
  "quality.wall": "Chute à cet endroit",
  "quality.bitsUsed": "Bits utilisés, mesurés",
  "quality.bitsOf": "{used} sur {declared}",
  "quality.bitsNote":
    "Une mesure des échantillons, pas une preuve de vraie haute résolution : un fichier qui utilise tous les bits qu’il annonce n’a pas été gonflé, ce qui ne veut pas dire qu’il a été enregistré à cette profondeur.",
  "quality.analysed": "Analysé le",
  "quality.chart": "Spectre moyen : les aigus s’arrêtent à {where}.",
  "quality.chartFullBand": "Spectre moyen : les aigus continuent sans mur.",
  "quality.showSpectrogram": "Afficher le spectrogramme",
  "quality.drawing": "Dessin en cours…",
  "quality.spectrogramAlt":
    "Spectrogramme de {title}, fréquence en fonction du temps",
  "quality.spectrogramBusy":
    "Un spectrogramme est déjà en cours de dessin pour vous. Réessayez dans un instant.",
  "quality.spectrogramFailed": "Le spectrogramme n’a pas pu être dessiné.",
  "quality.verdict.lossy_source": "Probablement issu d’un fichier avec perte",
  "quality.verdict.upsampled": "Probablement suréchantillonné",
  "quality.verdict.padded": "Profondeur gonflée",
  "quality.verdict.unreadable": "Illisible",
  "quality.verdict.silent": "Silencieux",
  "quality.verdict.pending": "En attente d’analyse",
  "quality.verdict.consistent": "Aucun signe de source avec perte",
  "quality.verdict.lossy": "Avec perte",
  "quality.verdict.not_measured": "Non mesuré",
  "quality.explain.lossy_source":
    "Les aigus s’arrêtent net là où coupe un encodeur avec perte : ce fichier a probablement été décodé depuis un MP3, un AAC ou un Opus, puis enregistré sans perte.",
  "quality.explain.upsampled":
    "La fréquence dépasse 48 kHz, mais les aigus s’arrêtent là où finit un CD ou un enregistrement à 48 kHz : il a probablement été rééchantillonné depuis l’un d’eux.",
  "quality.explain.padded":
    "Le conteneur annonce plus de bits que les échantillons n’en utilisent : un enregistrement 16 bits rangé en 24 bits, par exemple.",
  "quality.explain.unreadable":
    "FFmpeg n’a pas pu décoder ce fichier. Il sera réessayé quand le fichier changera.",
  "quality.explain.silent":
    "Rien dans ce fichier n’est assez fort pour être mesuré.",
  "quality.explain.pending":
    "Ce fichier n’a pas encore été analysé. L’analyse tourne en arrière-plan.",
  "quality.explain.consistent":
    "Rien dans le son ne contredit le conteneur. Ce n’est pas une preuve : un encodeur qui garde toute la bande ne laisse rien à trouver.",
  "quality.explain.lossy":
    "Un fichier avec perte dit ce qu’il est ; sa qualité est celle de son encodeur et de son débit. Il n’est pas analysé.",
  "quality.explain.not_measured": "Ce format n’est pas analysé.",
};
