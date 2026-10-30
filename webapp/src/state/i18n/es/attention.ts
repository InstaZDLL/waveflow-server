// The attention lists and artist splits.
import type { TranslationValue } from "..";
import type { attention as en } from "../en/attention";

export const attention: Record<keyof typeof en, TranslationValue> = {
  "attention.lists": "Listas",
  "attention.total": {
    one: "{count} cosa por revisar",
    other: "{count} cosas por revisar",
  },
  "attention.nothing": "No hay nada que revisar.",
  "attention.notYours":
    "Solo los propietarios y gestores de esta biblioteca ven lo que hay que revisar en ella.",
  "attention.empty": "No hay nada en esta lista.",
  "attention.tracksNote":
    "Corrige una pista desde su menú: una vez corregida, sale de la lista.",
  "attention.creditsNote":
    "Cada uno de estos créditos nombra a varios artistas en una sola cadena. Divídelo en los artistas que nombra, o mantenlo como uno solo si es un único nombre. Todos los escaneos posteriores aplican la decisión.",
  "attention.pending": "Por decidir",
  "attention.decided": "Decididos",
  "attention.split": "Dividir",
  "attention.confirmSplit": "Confirmar: dividir en {count}",
  "attention.keep": "Mantener como uno",
  "attention.splitDone": "{name} se ha dividido.",
  "attention.keepDone": "{name} se mantiene como uno solo.",
  "attention.undone": "Se deshizo la decisión sobre {name}.",
  "attention.alreadyDecided":
    "Alguien decidió sobre este crédito mientras tanto. La lista se ha recargado.",
  "attention.failed": "No funcionó. Vuelve a intentarlo.",
  "attention.noCredits": "Ningún crédito espera una decisión.",
  "attention.noDecisions": "Todavía no se ha decidido nada.",
  "attention.fragments": "En qué se dividiría {name}",
  "attention.knownFragment": "Ya es un artista de esta biblioteca",
  "attention.decidedSplit": "Dividido en {parts}",
  "attention.decidedKeep": "Mantenido como uno",
  "attention.decidedBy": "Por {who}, el {when}",
  "attention.undo": "Deshacer",
  "attention.duplicateGroup": "Posible duplicado {group}",
  "attention.split_credits": "Artistas por dividir",
  "attention.missing_artist": "Falta el artista",
  "attention.missing_album": "Falta el álbum",
  "attention.missing_year": "Falta el año",
  "attention.missing_track_number": "Falta el número de pista",
  "attention.missing_cover": "Falta la portada",
  "attention.album_year_conflict": "Álbumes con varios años",
  "attention.compilation_no_album_artist":
    "Recopilatorios sin artista del álbum",
  "attention.duplicate_track_number": "Números de pista repetidos",
  "attention.track_number_gap": "Huecos en los números de pista",
  "attention.probable_duplicate": "Posibles duplicados",
  "attention.probable_fake_lossless": "Posibles falsos sin pérdida",
  "artistSplit.offer": "Dividir en {count} artistas",
  "artistSplit.confirm": "Confirmar: dividir en {count}",
  "artistSplit.done": "Dividido en {parts}.",
  "artistSplit.failed": "No se pudo dividir este crédito. Vuelve a intentarlo.",
};
