// Playlists and their import, the queue, shares and bookmarks.
import type { TranslationValue } from "..";
import type { playlists as en } from "../en/playlists";

export const playlists: Record<keyof typeof en, TranslationValue> = {
  "playlists.detail": {
    one: "{count} colección",
    other: "{count} colecciones",
  },
  "playlists.newName": "Nombre de la nueva lista",
  "playlists.create": "Crear lista",
  "playlists.play": "Reproducir",
  "playlists.addQueue": "Añadir la cola",
  "playlists.emptyOne": "Esta lista de reproducción está vacía.",
  "playlists.export": "Guardar como .m3u8",
  "playlists.exportError": "No se pudo guardar esa lista de reproducción.",
  "playlists.empty":
    "Crea una lista de reproducción para mantener juntas un conjunto de pistas.",
  "playlists.createError": "No se pudo crear la lista de reproducción.",
  "playlists.queueError":
    "No se pudo añadir la cola a esta lista de reproducción.",
  "playlists.deleteError": "No se pudo eliminar la lista de reproducción.",
  "playlists.public": "Visible para todas las cuentas",
  "playlists.publicError": "No se pudo publicar esa lista de reproducción.",
  "playlists.others": "De otras cuentas",
  "playlists.othersDetail":
    "Listas de reproducción públicas de otras cuentas, con las pistas que tienen tus bibliotecas.",
  "playlists.by": "De {owner}",
  "playlists.moveUp": "Subir {title}",
  "playlists.moveDown": "Bajar {title}",
  "playlists.removeEntry": "Quitar {title} de esta lista",
  "playlists.moveError":
    "No se pudo mover esa entrada. Puede que la lista haya cambiado en otro sitio; se ha vuelto a cargar.",
  "playlists.removeEntryError": "No se pudo quitar esa entrada.",
  "import.title": "Traer listas de reproducción de otro reproductor",
  "import.detail":
    "Un archivo .m3u o .m3u8. No se crea nada hasta que hayas visto lo que se perdería.",
  "import.pick": "Elegir un archivo de lista",
  "import.name": "Nombre de la lista",
  "import.found": "{resolved} de {entries} entradas están en tu biblioteca.",
  "import.encoding":
    "El archivo no estaba en UTF-8 y se leyó como {encoding}. Revisa los nombres de abajo antes de importar.",
  "import.missing": {
    one: "{count} entrada no se puede ubicar:",
    other: "{count} entradas no se pueden ubicar:",
  },
  "import.line": "Línea {line}",
  "import.noMatch": "ninguna pista coincide",
  "import.ambiguous": "coincide con varias pistas",
  "import.notAPath": "no es un archivo de una biblioteca",
  "import.duplicate": "ya está en esta lista",
  "import.confirm": "Importar",
  "import.cancel": "Cancelar",
  "import.failed": "No se pudo leer ese archivo como lista de reproducción.",
  "queue.detail": {
    one: "{count} pista sincronizada con tu cuenta",
    other: "{count} pistas sincronizadas con tu cuenta",
  },
  "queue.playStart": "Reproducir desde el principio",
  "queue.clear": "Vaciar la cola",
  "queue.remove": "Quitar",
  "queue.empty":
    "Reproduce un álbum o una lista de reproducción para crear tu cola sincronizada.",
  "shares.detail": "Enlaces públicos a la música que elijas",
  "shares.description": "Descripción",
  "shares.create": "Compartir la cola actual",
  "shares.needQueue": "Añade pistas a la cola antes de crear un enlace.",
  "shares.once": "Este enlace solo se muestra una vez:",
  "shares.defaultName": "Música compartida",
  "shares.empty": "No se ha creado ningún enlace público.",
  "shares.createError": "No se pudo crear el enlace para compartir.",
  "shares.tooLong":
    "Un enlace para compartir admite hasta {limit} pistas, y la cola tiene {count}. Acorta la cola para compartirla.",
  "shares.deleteError": "No se pudo eliminar el enlace para compartir.",
  "shares.maxVisits": "Límite de visitas (opcional)",
  "shares.visits": { one: "{count} visita", other: "{count} visitas" },
  "shares.visitsOf": "{count} de {max} visitas",
  "share.kicker": "Compartido contigo",
  "share.defaultTitle": "Música compartida contigo",
  "share.until": "Disponible hasta el {date}",
  "share.empty": "Ya no se puede reproducir ninguna de estas pistas.",
  "share.player": "Reproductor",
  "share.unavailableTitle": "Este enlace no abre nada",
  "share.unavailableDetail":
    "Puede que haya caducado, que lo hayan retirado o que ya se haya abierto tantas veces como permite.",
  "bookmarks.title": "Posiciones guardadas",
  "bookmarks.detail": "Una por pista. Si vuelves a guardar, se mueve.",
  "bookmarks.save": "Guardar esta posición",
  "bookmarks.forget": "Olvidar",
};
