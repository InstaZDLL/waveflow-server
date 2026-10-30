// Sign-in and first-run setup.
import type { TranslationValue } from "..";
import type { login as en } from "../en/login";

export const login: Record<keyof typeof en, TranslationValue> = {
  "login.heroEyebrow": "Tu música, a tu manera",
  "login.heroTitle": "Al caer la noche, la biblioteca cobra vida.",
  "login.heroDetail":
    "Streaming privado, un solo catálogo, en todas tus pantallas.",
  "login.server": "Servidor WaveFlow",
  "login.createAdmin": "Crear el administrador",
  "login.welcome": "Hola de nuevo",
  "login.setupDetail":
    "Este servidor es nuevo. Elige la primera cuenta de administrador.",
  "login.setupToken": "Token de configuración",
  "login.setupTokenHint":
    "El servidor lo escribió en el archivo setup-token de su directorio de datos, y su registro indica la ruta. Si definiste WAVEFLOW_SETUP_TOKEN, introduce ese valor.",
  "login.setupTokenError":
    "Este token de configuración no es el correcto. Vuelve a copiarlo del archivo setup-token.",
  "login.username": "Nombre de usuario",
  "login.password": "Contraseña",
  "login.setupError":
    "No se pudo completar la configuración. Usa un nombre de usuario válido y una contraseña de al menos 12 caracteres.",
  "login.error": "Nombre de usuario o contraseña incorrectos.",
  "login.wait": "Espera un momento…",
  "login.create": "Crear e iniciar sesión",
  "login.signIn": "Iniciar sesión",
  "login.offline": {
    one: "Escuchar la {count} pista guardada en este navegador",
    other: "Escuchar las {count} pistas guardadas en este navegador",
  },
};
