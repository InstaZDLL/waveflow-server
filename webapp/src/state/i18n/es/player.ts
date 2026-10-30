// The player bar, now playing, radio and offline listening.
import type { TranslationValue } from "..";
import type { player as en } from "../en/player";

export const player: Record<keyof typeof en, TranslationValue> = {
  "player.error":
    "No se pudo iniciar la reproducción. Comprueba el servidor y vuelve a intentarlo.",
  "player.previous": "Pista anterior",
  "player.play": "Reproducir",
  "player.pause": "Pausa",
  "player.next": "Pista siguiente",
  "player.seek": "Buscar posición",
  "player.shuffle": "Aleatorio",
  "player.repeatOff": "Repetir: desactivado",
  "player.repeatAll": "Repetir: toda la cola",
  "player.repeatOne": "Repetir: esta pista",
  "player.volume": "Volumen",
  "player.mute": "Silenciar",
  "player.unmute": "Activar sonido",
  "player.openAlbum": "Abrir el álbum",
  "player.live": "En vivo",
  "player.liveRadio": "Radio en vivo",
  "player.stopLive": "Detener la emisora",
  "player.liveError": "No se pudo reproducir esta emisora.",
  "player.closeAside": "Detener y cerrar",
  "playing.idle": "No se está reproduciendo nada.",
  "playing.empty": "Pon una pista y aparecerá aquí, con su letra.",
  "playing.viewLabel": "Cómo ver esta pista",
  "playing.viewDetail": "Detalle",
  "playing.viewLyrics": "Letra",
  "playing.viewImmersive": "Inmersiva",
  "playing.immersiveTitle": "Vista inmersiva",
  "playing.leaveImmersive": "Salir de la vista inmersiva",
  "radio.detail": "Emisoras por internet, guardadas en tu cuenta",
  "radio.name": "Nombre de la emisora",
  "radio.address": "Dirección del stream",
  "radio.homepage": "Página de la emisora (opcional)",
  "radio.image": "Imagen de la emisora (opcional)",
  "radio.imageHint":
    "El servidor la descarga una sola vez, al guardar, y la conserva junto con las portadas: tu navegador nunca contacta con el sitio de la imagen.",
  "radio.imageMissing":
    "No se pudo obtener la imagen; guarda la emisora para volver a intentarlo.",
  "radio.add": "Añadir emisora",
  "radio.save": "Guardar emisora",
  "radio.edit": "Editar",
  "radio.replacing":
    "Al guardar se reemplazan todos los campos, así que una dirección de página vacía borra la guardada.",
  "radio.listen": "Escuchar",
  "radio.stop": "Detener",
  "radio.visit": "Página de la emisora",
  "radio.empty": "Todavía no hay emisoras. Añade una para tenerla aquí.",
  "radio.addressRefused": "La dirección debe empezar por http:// o https://.",
  "radio.mixedContent":
    "Esta página se sirve por HTTPS, así que tu navegador rechazará un stream sin cifrar.",
  "radio.full": "Esta cuenta ya tiene el máximo de emisoras permitido.",
  "radio.saveError": "No se pudo guardar la emisora.",
  "radio.deleteError": "No se pudo eliminar la emisora.",
  "radio.playbackError":
    "No se pudo reproducir esa emisora. El servidor nunca se conecta a ella, así que el problema está entre tu navegador y la emisora.",
  "radio.stations": "Emisoras",
  "radio.newTitle": "Añadir una emisora",
  "radio.editTitle": "Editar “{name}”",
  "radio.listenTo": "Escuchar {name}",
  "radio.stopNamed": "Detener {name}",
  "radio.actions": "Acciones de {name}",
  "radio.confirmDelete":
    "¿Eliminar la emisora “{name}”? Se olvidan su dirección y su imagen.",
  "offline.kicker": "En este navegador",
  "offline.title": "Guardado sin conexión",
  "offline.detail":
    "Estas pistas se guardaron en este navegador para la última cuenta que inició sesión, y se reproducen sin conexión. Al cerrar sesión se eliminan de este navegador. Guarda más desde el menú de una pista.",
  "offline.unsupported":
    "Este navegador no puede guardar pistas para escucharlas sin conexión.",
  "offline.back": "Volver a la biblioteca",
  "offline.empty":
    "Todavía no hay nada guardado. El menú de una pista te ofrece guardarla.",
  "offline.player": "Reproductor sin conexión",
  "offline.gone": "Esa copia ya no está en este navegador.",
  "offline.forget": "Quitar la copia sin conexión de {title}",
  "offline.forgetShort": "Quitar",
  "offline.forgetDetail": "Libera {size}",
};
