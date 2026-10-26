// Sign-in and first-run setup.
import type { TranslationValue } from "..";
import type { login as en } from "../en/login";

export const login: Record<keyof typeof en, TranslationValue> = {
  "login.heroEyebrow": "Votre musique, selon vos règles",
  "login.heroTitle": "La nuit tombe, la bibliothèque s’éveille.",
  "login.heroDetail": "Streaming privé, un catalogue, tous vos écrans.",
  "login.server": "Serveur WaveFlow",
  "login.createAdmin": "Créer le compte administrateur",
  "login.welcome": "Bon retour",
  "login.setupDetail":
    "Ce serveur est neuf. Choisissez son premier compte administrateur.",
  "login.setupToken": "Jeton d’installation",
  "login.setupTokenHint":
    "Le serveur l’a écrit dans le fichier setup-token de son répertoire de données, dont son journal donne le chemin. Si vous avez défini WAVEFLOW_SETUP_TOKEN, saisissez cette valeur.",
  "login.setupTokenError":
    "Ce jeton d’installation n’est pas le bon. Copiez-le de nouveau depuis le fichier setup-token.",
  "login.username": "Nom d’utilisateur",
  "login.password": "Mot de passe",
  "login.setupError":
    "Configuration impossible. Utilisez un nom valide et un mot de passe d’au moins 12 caractères.",
  "login.error": "Nom d’utilisateur ou mot de passe incorrect.",
  "login.wait": "Veuillez patienter…",
  "login.create": "Créer et se connecter",
  "login.signIn": "Se connecter",
  "login.offline": {
    one: "Écouter le {count} titre gardé dans ce navigateur",
    other: "Écouter les {count} titres gardés dans ce navigateur",
  },
};
