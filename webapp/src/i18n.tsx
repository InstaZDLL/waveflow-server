import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useState,
} from "react";

const en = {
  "nav.albums": "Albums",
  "nav.artists": "Artists",
  "nav.search": "Search",
  "nav.favourites": "Favourites",
  "nav.playlists": "Playlists",
  "nav.queue": "Queue",
  "nav.shares": "Shares",
  "nav.admin": "Admin",
  "nav.signOut": "Sign out",
  "nav.skip": "Skip to content",
  "nav.home": "WaveFlow home",
  "nav.server": "server",
  "preferences.theme": "Theme",
  "preferences.language": "Language",
  "common.loading": "Loading",
  "common.loadError": "We could not load this view",
  "common.noAlbums": "No albums yet.",
  "common.noArtists": "No artists yet.",
  "common.variousArtists": "Various artists",
  "common.unknownArtist": "Unknown artist",
  "common.album": "Album",
  "common.artist": "Artist",
  "common.albums": "{count} albums",
  "common.tracks": "{count} tracks",
  "common.delete": "Delete",
  "common.cancel": "Cancel",
  "login.heroEyebrow": "Your music, on your terms",
  "login.heroTitle": "After dark, the library comes alive.",
  "login.heroDetail": "Private streaming, one catalogue, every screen.",
  "login.server": "WaveFlow server",
  "login.createAdmin": "Create the administrator",
  "login.welcome": "Welcome back",
  "login.setupDetail":
    "This is a new server. Choose the first administrator account.",
  "login.username": "Username",
  "login.password": "Password",
  "login.setupError":
    "Setup failed. Use a valid username and at least 12 password characters.",
  "login.error": "Wrong username or password.",
  "login.wait": "Please wait…",
  "login.create": "Create and sign in",
  "login.signIn": "Sign in",
  "albums.detail": "{count} records in your library",
  "artists.detail": "{count} voices and collaborators",
  "artists.libraryCount": "{count} albums in your library",
  "favourites.detail": "{count} saved tracks",
  "favourites.empty":
    "Star a track from an album or search result and it will appear here.",
  "favourites.add": "Add favourite",
  "favourites.remove": "Remove favourite",
  "playlists.detail": "{count} collections",
  "playlists.newName": "New playlist name",
  "playlists.create": "Create playlist",
  "playlists.play": "Play",
  "playlists.addQueue": "Add queue",
  "playlists.emptyOne": "This playlist is empty.",
  "playlists.empty": "Create a playlist to keep a set of tracks together.",
  "playlists.createError": "The playlist could not be created.",
  "playlists.queueError": "The queue could not be added to this playlist.",
  "playlists.deleteError": "The playlist could not be deleted.",
  "queue.detail": "{count} tracks synchronized with your account",
  "queue.playStart": "Play from start",
  "queue.clear": "Clear queue",
  "queue.remove": "Remove",
  "queue.empty": "Play an album or playlist to build your synchronized queue.",
  "shares.detail": "Public links for selected music",
  "shares.description": "Description",
  "shares.create": "Share current queue",
  "shares.needQueue": "Add tracks to the queue before creating a link.",
  "shares.once": "This link is shown only once:",
  "shares.defaultName": "Music share",
  "shares.empty": "No public links have been created.",
  "shares.createError": "The share could not be created.",
  "shares.deleteError": "The share could not be deleted.",
  "credential.password": "Dedicated Subsonic password",
  "credential.rotate": "Rotate credential",
  "credential.copy": "Copy this API key now:",
  "credential.error": "Credential rotation failed.",
  "admin.title": "Administration",
  "admin.detail": "Libraries, scans and accounts",
  "admin.libraries": "Libraries",
  "admin.libraryName": "Library name",
  "admin.libraryPath": "Absolute server folder path",
  "admin.registering": "Registering…",
  "admin.register": "Register and scan",
  "admin.scan": "Scan now",
  "admin.accounts": "Accounts",
  "admin.webPassword": "Web password, 12 characters minimum",
  "admin.creating": "Creating…",
  "admin.create": "Create account",
  "admin.enable": "Enable",
  "admin.disable": "Disable",
  "admin.currentAccount": "You cannot disable your current account",
  "admin.libraryError": "The library path could not be registered.",
  "admin.accountError": "The account could not be created.",
  "admin.statusError": "The account status could not be changed.",
  "admin.scanError": "The scan could not be started.",
  "admin.scanQueued": "Scan {id} queued.",
  "admin.initialScan": "Initial scan {id} queued.",
  "search.detail": "Find a title, album or artist",
  "search.placeholder": "Title, album or artist",
  "search.query": "Search query",
  "search.tracks": "Tracks",
  "search.empty": "Nothing found.",
  "player.error": "Playback could not start. Check the server and try again.",
  "player.previous": "Previous track",
  "player.play": "Play",
  "player.pause": "Pause",
  "player.next": "Next track",
  "player.seek": "Seek",
  "authorize.title": "Authorisation",
  "authorize.invalid":
    "This authorisation link is incomplete or asks to send you somewhere we do not trust.",
  "authorize.client": "Authorise {client}",
  "authorize.detail":
    "It will be able to browse your libraries, play your music and manage your playlists, favourites and ratings, everything this account can do.",
  "authorize.redirect": "Sending you back to {url}",
  "authorize.error":
    "The application sent an authorisation request we cannot honour.",
  "authorize.busy": "Authorising…",
  "authorize.approve": "Authorise",
  "notFound.title": "This room is silent",
  "notFound.detail": "The page you asked for does not exist on this server.",
  "notFound.back": "Back to the library",
} as const;

type TranslationKey = keyof typeof en;

const fr: Record<TranslationKey, string> = {
  "nav.albums": "Albums",
  "nav.artists": "Artistes",
  "nav.search": "Recherche",
  "nav.favourites": "Favoris",
  "nav.playlists": "Playlists",
  "nav.queue": "File d’attente",
  "nav.shares": "Partages",
  "nav.admin": "Administration",
  "nav.signOut": "Se déconnecter",
  "nav.skip": "Aller au contenu",
  "nav.home": "Accueil WaveFlow",
  "nav.server": "serveur",
  "preferences.theme": "Thème",
  "preferences.language": "Langue",
  "common.loading": "Chargement",
  "common.loadError": "Impossible de charger cette vue",
  "common.noAlbums": "Aucun album pour le moment.",
  "common.noArtists": "Aucun artiste pour le moment.",
  "common.variousArtists": "Artistes variés",
  "common.unknownArtist": "Artiste inconnu",
  "common.album": "Album",
  "common.artist": "Artiste",
  "common.albums": "{count} albums",
  "common.tracks": "{count} pistes",
  "common.delete": "Supprimer",
  "common.cancel": "Annuler",
  "login.heroEyebrow": "Votre musique, selon vos règles",
  "login.heroTitle": "La nuit tombe, la bibliothèque s’éveille.",
  "login.heroDetail": "Streaming privé, un catalogue, tous vos écrans.",
  "login.server": "Serveur WaveFlow",
  "login.createAdmin": "Créer le compte administrateur",
  "login.welcome": "Bon retour",
  "login.setupDetail":
    "Ce serveur est neuf. Choisissez son premier compte administrateur.",
  "login.username": "Nom d’utilisateur",
  "login.password": "Mot de passe",
  "login.setupError":
    "Configuration impossible. Utilisez un nom valide et un mot de passe d’au moins 12 caractères.",
  "login.error": "Nom d’utilisateur ou mot de passe incorrect.",
  "login.wait": "Veuillez patienter…",
  "login.create": "Créer et se connecter",
  "login.signIn": "Se connecter",
  "albums.detail": "{count} disques dans votre bibliothèque",
  "artists.detail": "{count} voix et collaborations",
  "artists.libraryCount": "{count} albums dans votre bibliothèque",
  "favourites.detail": "{count} pistes enregistrées",
  "favourites.empty":
    "Ajoutez une piste aux favoris depuis un album ou la recherche pour la retrouver ici.",
  "favourites.add": "Ajouter aux favoris",
  "favourites.remove": "Retirer des favoris",
  "playlists.detail": "{count} collections",
  "playlists.newName": "Nom de la nouvelle playlist",
  "playlists.create": "Créer la playlist",
  "playlists.play": "Lire",
  "playlists.addQueue": "Ajouter la file",
  "playlists.emptyOne": "Cette playlist est vide.",
  "playlists.empty": "Créez une playlist pour réunir plusieurs pistes.",
  "playlists.createError": "La playlist n’a pas pu être créée.",
  "playlists.queueError": "La file n’a pas pu être ajoutée à cette playlist.",
  "playlists.deleteError": "La playlist n’a pas pu être supprimée.",
  "queue.detail": "{count} pistes synchronisées avec votre compte",
  "queue.playStart": "Lire depuis le début",
  "queue.clear": "Vider la file",
  "queue.remove": "Retirer",
  "queue.empty": "Lisez un album ou une playlist pour construire votre file.",
  "shares.detail": "Liens publics vers votre musique",
  "shares.description": "Description",
  "shares.create": "Partager la file actuelle",
  "shares.needQueue": "Ajoutez des pistes à la file avant de créer un lien.",
  "shares.once": "Ce lien ne sera affiché qu’une fois :",
  "shares.defaultName": "Partage musical",
  "shares.empty": "Aucun lien public n’a été créé.",
  "shares.createError": "Le partage n’a pas pu être créé.",
  "shares.deleteError": "Le partage n’a pas pu être supprimé.",
  "credential.password": "Mot de passe Subsonic dédié",
  "credential.rotate": "Renouveler l’identifiant",
  "credential.copy": "Copiez cette clé API maintenant :",
  "credential.error": "Le renouvellement de l’identifiant a échoué.",
  "admin.title": "Administration",
  "admin.detail": "Bibliothèques, scans et comptes",
  "admin.libraries": "Bibliothèques",
  "admin.libraryName": "Nom de la bibliothèque",
  "admin.libraryPath": "Chemin absolu du dossier sur le serveur",
  "admin.registering": "Ajout en cours…",
  "admin.register": "Ajouter et scanner",
  "admin.scan": "Scanner",
  "admin.accounts": "Comptes",
  "admin.webPassword": "Mot de passe web, 12 caractères minimum",
  "admin.creating": "Création…",
  "admin.create": "Créer le compte",
  "admin.enable": "Activer",
  "admin.disable": "Désactiver",
  "admin.currentAccount": "Vous ne pouvez pas désactiver votre compte actuel",
  "admin.libraryError": "Le dossier n’a pas pu être enregistré.",
  "admin.accountError": "Le compte n’a pas pu être créé.",
  "admin.statusError": "Le statut du compte n’a pas pu être modifié.",
  "admin.scanError": "Le scan n’a pas pu démarrer.",
  "admin.scanQueued": "Scan {id} ajouté à la file.",
  "admin.initialScan": "Scan initial {id} ajouté à la file.",
  "search.detail": "Trouvez un titre, un album ou un artiste",
  "search.placeholder": "Titre, album ou artiste",
  "search.query": "Requête de recherche",
  "search.tracks": "Pistes",
  "search.empty": "Aucun résultat.",
  "player.error": "Lecture impossible. Vérifiez le serveur puis réessayez.",
  "player.previous": "Piste précédente",
  "player.play": "Lire",
  "player.pause": "Pause",
  "player.next": "Piste suivante",
  "player.seek": "Position de lecture",
  "authorize.title": "Autorisation",
  "authorize.invalid":
    "Ce lien d’autorisation est incomplet ou mène vers une destination non fiable.",
  "authorize.client": "Autoriser {client}",
  "authorize.detail":
    "Cette application pourra parcourir vos bibliothèques, lire votre musique et gérer vos playlists, favoris et notes, comme ce compte.",
  "authorize.redirect": "Retour vers {url}",
  "authorize.error": "La demande d’autorisation de l’application est invalide.",
  "authorize.busy": "Autorisation…",
  "authorize.approve": "Autoriser",
  "notFound.title": "Cette salle est silencieuse",
  "notFound.detail": "La page demandée n’existe pas sur ce serveur.",
  "notFound.back": "Retour à la bibliothèque",
};

export type Locale = "en" | "fr";

type I18n = {
  locale: Locale;
  setLocale: (locale: Locale) => void;
  t: (key: TranslationKey, values?: Record<string, string | number>) => string;
};

const I18nContext = createContext<I18n | null>(null);
const STORAGE_KEY = "waveflow.locale";

export function I18nProvider({ children }: { children: ReactNode }) {
  const [locale, setLocale] = useState<Locale>(() => {
    try {
      const saved = localStorage.getItem(STORAGE_KEY);
      if (saved === "en" || saved === "fr") return saved;
    } catch {
      // Fall back to the browser language when storage is unavailable.
    }
    return navigator.language.toLowerCase().startsWith("fr") ? "fr" : "en";
  });

  useEffect(() => {
    document.documentElement.lang = locale;
    try {
      localStorage.setItem(STORAGE_KEY, locale);
    } catch {
      // Language switching remains available without persistence.
    }
  }, [locale]);

  const t: I18n["t"] = useCallback(
    (key, values = {}) => {
      let value: string = (locale === "fr" ? fr : en)[key];
      for (const [name, replacement] of Object.entries(values)) {
        value = value.replaceAll(`{${name}}`, String(replacement));
      }
      return value;
    },
    [locale],
  );

  return (
    <I18nContext.Provider value={{ locale, setLocale, t }}>
      {children}
    </I18nContext.Provider>
  );
}

export function useI18n(): I18n {
  const value = useContext(I18nContext);
  if (!value) throw new Error("useI18n requires I18nProvider");
  return value;
}

export function LanguagePicker() {
  const { locale, setLocale, t } = useI18n();
  return (
    <label className="language-picker">
      <span>{t("preferences.language")}</span>
      <select
        aria-label={t("preferences.language")}
        value={locale}
        onChange={(event) => setLocale(event.target.value as Locale)}
      >
        <option value="en">English</option>
        <option value="fr">Français</option>
      </select>
    </label>
  );
}
