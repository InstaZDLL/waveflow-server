// Folders, filing and uploads.
import type { TranslationValue } from "..";
import type { folders as en } from "../en/folders";

export const folders: Record<keyof typeof en, TranslationValue> = {
  "folders.count": {
    one: "{count} carpeta creada en esta biblioteca",
    other: "{count} carpetas creadas en esta biblioteca",
  },
  "folders.where": "Ruta de la carpeta",
  "folders.subfolders": "Carpetas",
  "folders.noLibrary": "Esta cuenta no tiene ninguna biblioteca disponible.",
  "folders.none": "Todavía no se ha creado ninguna carpeta en esta biblioteca.",
  "folders.system": "Gestionada por WaveFlow",
  "folders.systemNote":
    "Por clasificar reúne todas las pistas que no están en ninguna carpeta. WaveFlow la gestiona por sí mismo: no cuenta entre las carpetas de arriba y no se puede renombrar, mover ni eliminar.",
  "filed.unfiled": "Por clasificar",
  "filed.automatic": "Automática",
  "filed.topLevel": "Nivel superior",
  "filed.here": "Aquí ahora",
  "filed.destinations": "Carpetas disponibles",
  "filed.loadError": "No se pudieron cargar las carpetas.",
  "filed.moveFailed": "No se pudo mover. Vuelve a intentarlo.",
  "filed.moveTrack": "Mover “{title}” a una carpeta",
  "filed.moveFolder": "Mover “{name}”",
  "filed.note":
    "Carpetas que solo existen en WaveFlow. Clasificar una pista aquí no mueve ningún archivo.",
  "filed.newFolder": "Nueva carpeta",
  "filed.name": "Nombre de la carpeta",
  "filed.renameLabel": "Nuevo nombre",
  "filed.create": "Crear",
  "filed.save": "Guardar",
  "filed.cancel": "Cancelar",
  "filed.rename": "Renombrar",
  "filed.move": "Mover…",
  "filed.delete": "Eliminar",
  "filed.confirmDelete":
    "¿Eliminar “{name}” y sus subcarpetas? Sus pistas vuelven a Por clasificar; no se toca ningún archivo.",
  "filed.created": "Se creó “{name}”.",
  "filed.renamed": "Renombrada como “{name}”.",
  "filed.deleted": {
    one: "Carpeta eliminada: {count} pista vuelve a Por clasificar.",
    other: "Carpeta eliminada: {count} pistas vuelven a Por clasificar.",
  },
  "filed.filed": {
    one: "{count} pista clasificada en {folder}.",
    other: "{count} pistas clasificadas en {folder}.",
  },
  "filed.folderMoved": "Se movió “{name}”.",
  "filed.nameTaken": "Aquí ya hay una carpeta con ese nombre.",
  "filed.refused":
    "Esa carpeta no puede ir ahí: revisa su nombre, o el árbol es demasiado profundo o está lleno.",
  "filed.failed": "No funcionó. Vuelve a intentarlo.",
  "filed.missing": "Esta carpeta ya no existe.",
  "filed.empty": "Todavía no hay nada clasificado aquí.",
  "filed.allFiled": "Todas las pistas están clasificadas.",
  "filed.uploadHere": "Subir aquí",
  "filed.unfiledNote":
    "Todas las pistas que aún no están en ninguna carpeta. WaveFlow gestiona esta lista por sí mismo: una pista sale de ella en cuanto se clasifica, y la lista no se puede renombrar ni eliminar. Para clasificar una pista, abre su menú y elige “Mover a una carpeta…”.",
  "filed.unfiledNoteReader":
    "Todas las pistas que aún no están en ninguna carpeta. WaveFlow gestiona esta lista por sí mismo: una pista sale de ella en cuanto se clasifica.",
  "upload.into": "Los archivos recibidos van a la carpeta {folder}.",
  "upload.doneUnfiled": "Añadido a la biblioteca, en Por clasificar",
  "upload.folderMissing":
    "No se encuentra la carpeta para la que se abrió esta subida: los archivos recibidos irán a Por clasificar.",
  "upload.folderUnchecked":
    "No se pudo comprobar la carpeta para la que se abrió esta subida, así que todavía no se acepta ningún archivo.",
  "upload.checkAgain": "Volver a comprobar",
  "upload.title": "Subir a {library}",
  "upload.detail":
    "Primero se calcula la huella de cada archivo en tu navegador, así que nunca se vuelve a enviar un archivo que la biblioteca ya tiene, y una transferencia interrumpida continúa donde se quedó.",
  "upload.pick": "Elegir archivos de audio",
  "upload.formats": "Formatos que la biblioteca puede indexar: {extensions}",
  "upload.noLibrary": "Elige primero una biblioteca.",
  "upload.notAllowed":
    "Solo un propietario o un gestor de esta biblioteca puede añadirle archivos.",
  "upload.closed":
    "Esta biblioteca no acepta archivos. Lo decide quien administra el servidor.",
  "upload.waiting": "En espera",
  "upload.hashing": "Calculando la huella…",
  "upload.negotiating": "Consultando al servidor…",
  "upload.uploading": "Enviando…",
  "upload.committing": "Añadiendo a la biblioteca…",
  "upload.done": "Añadido a la biblioteca",
  "upload.present": "Ya está en la biblioteca",
  "upload.refused": "Rechazado: {reason}",
  "upload.failed": "Falló: {reason}",
  "upload.progress": "Progreso: {name}",
  "upload.reason.unsupportedFormat":
    "no es un formato de audio que la biblioteca pueda indexar",
  "upload.reason.tooLarge": "es más grande de lo que acepta el servidor",
  "upload.reason.quotaExceeded": "la biblioteca no tiene más espacio",
  "upload.reason.libraryClosed": "la biblioteca no acepta archivos",
  "upload.reason.tooManySessions":
    "ya hay demasiadas transferencias abiertas; vuelve a intentarlo en un momento",
  "upload.reason.refusedFile":
    "lo que llegó no coincide con el archivo, o no es un audio que la biblioteca pueda leer",
  "upload.reason.unavailable":
    "el servidor no pudo aceptarlo en este momento; vuelve a intentarlo",
  "upload.reason.network": "se perdió la conexión",
  "upload.reason.notAllowed": "ya no puedes añadir archivos a esta biblioteca",
  "upload.reason.error": "algo salió mal",
  "upload.reason.hashing":
    "este navegador no pudo calcular la huella del archivo",
};
