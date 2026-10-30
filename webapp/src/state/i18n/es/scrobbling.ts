// Scrobbling.
import type { TranslationValue } from "..";
import type { scrobbling as en } from "../en/scrobbling";

export const scrobbling: Record<keyof typeof en, TranslationValue> = {
  "scrobbling.title": "Scrobbling externo",
  "scrobbling.detail":
    "Envía lo que escuchas aquí a ListenBrainz, Maloja o Last.fm.",
  "scrobbling.none":
    "Este servidor no ofrece ningún destino. Lo decide quien lo administra.",
  "scrobbling.provider.listenbrainz": "ListenBrainz",
  "scrobbling.provider.maloja": "Maloja",
  "scrobbling.provider.lastfm": "Last.fm",
  "scrobbling.instance": "instancia “{name}”",
  "scrobbling.unavailable": "No disponible aquí: {reason}",
  "scrobbling.unavailable.no_application_configured":
    "no hay ninguna aplicación de Last.fm declarada en este servidor",
  "scrobbling.unavailable.browser_journey_needs_https":
    "la conexión desde el navegador necesita que la dirección pública de este servidor sea https; quien lo administra aún puede vincular una cuenta desde la línea de comandos",
  "scrobbling.unavailable.unknown": "este servidor no indicó el motivo",
  "scrobbling.retired":
    "Este servidor ya no ofrece esta instancia. No se le enviará nada más; aún puedes retirar la autorización.",
  "scrobbling.notLinked": "Sin vincular.",
  "scrobbling.secret": "Clave de API",
  "scrobbling.secretHint": "Se obtiene de tu cuenta en ese destino.",
  "scrobbling.link": "Vincular",
  "scrobbling.working": "Procesando…",
  "scrobbling.linkError":
    "Se rechazó esa autorización. Revisa la clave y vuelve a intentarlo.",
  "scrobbling.unlink": "Desvincular",
  "scrobbling.unlinkError": "No se pudo retirar ese vínculo.",
  "scrobbling.connect": "Conectar con Last.fm",
  "scrobbling.connectDetail":
    "Last.fm no da ninguna clave que puedas pegar, así que te lo pide en su sitio y te trae de vuelta.",
  "scrobbling.connectError": "No se pudo abrir la conexión con Last.fm.",
  "scrobbling.returned": "Last.fm está vinculado: {name}.",
  "scrobbling.health.healthy": "Las escuchas se están enviando",
  "scrobbling.health.degraded": "La cola no avanza",
  "scrobbling.health.broken": "Roto",
  "scrobbling.healthDetail.healthy": "No hay nada atascado.",
  "scrobbling.healthDetail.degraded":
    "El vínculo responde, pero las escuchas no llegan: o hay una esperando tu decisión, o la más antigua lleva demasiado tiempo esperando.",
  "scrobbling.healthDetail.broken":
    "Se rechazó la credencial, o este servidor ya no puede llegar a esa instancia con el nombre con el que se creó el vínculo.",
  "scrobbling.pending": {
    one: "{count} en espera",
    other: "{count} en espera",
  },
  "scrobbling.retrying": {
    one: "{count} en reintento",
    other: "{count} en reintento",
  },
  "scrobbling.uncertainCount": {
    one: "{count} espera tu decisión",
    other: "{count} esperan tu decisión",
  },
  "scrobbling.oldest": "La más antigua espera desde el {when}.",
  "scrobbling.lastSuccess": "Última aceptada el {when}.",
  "scrobbling.lastFailure": "Último rechazo: {reason}.",
  "scrobbling.uncertain": "Escuchas que solo tú puedes decidir",
  "scrobbling.uncertainDetail":
    "La conexión se cortó después de que salieran, así que nadie sabe si el destino las registró. Volver a enviar una puede hacer que aparezca dos veces en un historial de escuchas que otras personas leen; descartarla deja un hueco. Este servidor no elegirá por ti, y solo te lo preguntará una vez.",
  "scrobbling.uncertainNone": "No hay nada esperando tu decisión.",
  "scrobbling.playedAt": "Reproducida el {when}",
  "scrobbling.attempts": { one: "{count} intento", other: "{count} intentos" },
  "scrobbling.retry": "Volver a enviar",
  "scrobbling.discard": "Descartar",
  "scrobbling.answered": "Respondido.",
  "scrobbling.answerError":
    "No se pudo responder sobre esa escucha. Recarga la página antes de volver a intentarlo.",
  "scrobbling.unnamedTrack":
    "Ya no está en el historial que conserva esta cuenta",
  "scrobbling.reason.rate_limited": "el destino pidió que se esperara",
  "scrobbling.reason.retryable": "el destino no la aceptó",
  "scrobbling.reason.auth_broken": "se rechazó la autorización",
  "scrobbling.reason.rejected": "el destino la leyó y la rechazó",
  "scrobbling.reason.destination_gone":
    "este servidor ya no llega a esa instancia con ese nombre",
  "scrobbling.reason.credential_unreadable":
    "este servidor no pudo leer la credencial guardada",
  "scrobbling.reason.attempts_exhausted": "se intentó demasiadas veces",
  "scrobbling.reason.unlinked": "se retiró el vínculo",
  "scrobbling.reason.ambiguous": "la respuesta nunca llegó",
  "scrobbling.reason.interrupted": "el servidor se detuvo a mitad del envío",
};
