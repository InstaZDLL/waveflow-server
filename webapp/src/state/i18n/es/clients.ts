// Registered clients, the Subsonic password and OAuth consent.
import type { TranslationValue } from "..";
import type { clients as en } from "../en/clients";

export const clients: Record<keyof typeof en, TranslationValue> = {
  "clients.title": "Clientes registrados",
  "clients.detail":
    "Dispositivos y scripts que tienen una credencial sobre tu biblioteca. Si revocas una, deja de funcionar al instante.",
  "clients.name": "Para qué es",
  "clients.register": "Registrar",
  "clients.once":
    "Cópiala ahora. Solo se guarda su hash, así que se muestra una vez y nunca más:",
  "clients.revoke": "Revocar",
  "clients.none": "Ningún cliente tiene una credencial sobre tu biblioteca.",
  "clients.error": "No funcionó. No se ha cambiado nada.",
  "clients.registered": "registrado el {when}",
  "clients.used": "usado por última vez el {when}",
  "clients.unused": "nunca usado",
  "subsonic.title": "Contraseña de Subsonic",
  "subsonic.detail":
    "La contraseña aparte con la que inician sesión los clientes de Subsonic. Al cambiarla también se reemplaza la clave de API, así que una que se haya filtrado deja de funcionar.",
  "subsonic.set": "definida el {when}",
  "subsonic.changed": "cambiada por última vez el {when}",
  "subsonic.password": "Nueva contraseña",
  "subsonic.rotate": "Cambiarla",
  "subsonic.revoke": "Desactivarla",
  "subsonic.confirm": "Sí, desactivarla",
  "subsonic.once":
    "Copia esta clave de API ahora. Solo se guarda su hash, así que se muestra una vez y nunca más:",
  "subsonic.gone":
    "Tus clientes de Subsonic dejarán de funcionar hasta que vuelvas a configurarla en cada uno.",
  "subsonic.none":
    "No tienes contraseña de Subsonic, así que ningún cliente de Subsonic puede leer tu biblioteca. Pídele una al administrador.",
  "subsonic.short": "Usa al menos 12 caracteres.",
  "subsonic.error": "No funcionó. No se ha cambiado nada.",
  "credential.password": "Contraseña exclusiva de Subsonic",
  "credential.rotate": "Renovar la credencial",
  "credential.copy": "Copia esta clave de API ahora:",
  "credential.error": "No se pudo renovar la credencial.",
  "authorize.title": "Autorización",
  "authorize.invalid":
    "Este enlace de autorización está incompleto o quiere enviarte a un sitio en el que no confiamos.",
  "authorize.client": "Autorizar a {client}",
  "authorize.detail":
    "Podrá explorar tus bibliotecas, reproducir tu música y gestionar tus listas de reproducción, favoritos y valoraciones: todo lo que puede hacer esta cuenta.",
  "authorize.redirect": "Te estamos enviando de vuelta a {url}",
  "authorize.error":
    "La aplicación envió una solicitud de autorización que no podemos atender.",
  "authorize.busy": "Autorizando…",
  "authorize.approve": "Autorizar",
};
