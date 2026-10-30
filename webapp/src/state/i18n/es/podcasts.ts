// Podcast subscriptions and episodes.
import type { TranslationValue } from "..";
import type { podcasts as en } from "../en/podcasts";

export const podcasts: Record<keyof typeof en, TranslationValue> = {
  "podcasts.detail":
    "Programas que sigue este servidor, para todas las cuentas",
  "podcasts.off":
    "Los pódcasts están desactivados en este servidor. Lo que se guardó antes sigue disponible; no se descarga nada nuevo.",
  "podcasts.url": "Dirección del feed",
  "podcasts.shared":
    "Visible para todas las cuentas: cualquiera en este servidor ve el programa y puede reproducir los episodios que descarga. Solo los administradores ven la dirección del feed.",
  "podcasts.subscribe": "Suscribirse",
  "podcasts.refresh": "Actualizar todos los feeds",
  "podcasts.refreshing": "Se están volviendo a descargar los feeds.",
  "podcasts.reload": "Recargar",
  "podcasts.empty": "Todavía no hay pódcasts.",
  "podcasts.emptyDetail":
    "Un administrador se suscribe aquí al feed de un programa. El servidor descarga sus episodios, y todas las cuentas de este servidor pueden escucharlos.",
  "podcasts.addFirst": "Añade tu primer pódcast",
  "podcasts.channels": "Programas",
  "podcasts.showing": "Mostrando los episodios de {title}.",
  "podcasts.noEpisodes": "Este feed no tiene episodios.",
  "podcasts.episodes": {
    one: "{count} episodio",
    other: "{count} episodios",
  },
  "podcasts.failed": "la última descarga falló: {reason}",
  "podcasts.open": "Episodios",
  "podcasts.hide": "Ocultar",
  "podcasts.unsubscribe": "Cancelar suscripción",
  "podcasts.download": "Descargar",
  "podcasts.play": "Reproducir {title}",
  "podcasts.pause": "Pausar {title}",
  "podcasts.resumeAt": "continúa en {time}",
  "podcasts.status.new": "sin descargar",
  "podcasts.status.downloading": "descargando",
  "podcasts.status.completed": "listo",
  "podcasts.status.error": "la descarga falló",
  "podcasts.status.deleted": "eliminado",
  "podcasts.conflict": "Este servidor ya sigue ese feed.",
  "podcasts.refused":
    "No se pudo descargar o leer el feed, o su dirección no es una a la que este servidor pueda acceder.",
  "podcasts.subscribeError": "No se pudo añadir el feed.",
  "podcasts.mutationError": "No funcionó. Vuelve a intentarlo.",
  "podcasts.playbackError": "No se pudo reproducir este episodio.",
};
