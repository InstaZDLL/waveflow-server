// Sign-in and first-run setup.
import type { TranslationValue } from "..";
import type { login as en } from "../en/login";

export const login: Record<keyof typeof en, TranslationValue> = {
  "login.heroEyebrow": "La tua musica, alle tue condizioni",
  "login.heroTitle": "Cala la notte, la libreria si risveglia.",
  "login.heroDetail": "Streaming privato, un solo catalogo, su ogni schermo.",
  "login.server": "Server WaveFlow",
  "login.createAdmin": "Crea l’amministratore",
  "login.welcome": "Ti diamo il bentornato",
  "login.setupDetail":
    "Questo è un nuovo server. Crea il primo account amministratore.",
  "login.setupToken": "Token di configurazione",
  "login.setupTokenHint":
    "Il server l’ha scritto nel file setup-token della sua cartella dati, e il suo log ne indica il percorso. Se hai impostato WAVEFLOW_SETUP_TOKEN, inserisci quel valore.",
  "login.setupTokenError":
    "Questo token di configurazione non è quello giusto. Copialo di nuovo dal file setup-token.",
  "login.username": "Nome utente",
  "login.password": "Password",
  "login.setupError":
    "Configurazione non riuscita. Usa un nome utente valido e una password di almeno 12 caratteri.",
  "login.error": "Nome utente o password errati.",
  "login.wait": "Attendi…",
  "login.create": "Crea e accedi",
  "login.signIn": "Accedi",
  "login.offline": {
    one: "Ascolta {count} brano salvato in questo browser",
    other: "Ascolta i {count} brani salvati in questo browser",
  },
};
