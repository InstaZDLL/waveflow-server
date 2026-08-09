# M3 — Subsonic/OpenSubsonic : état & handoff

> Mise à jour (2026-08-09). La porte M3 est **fermée** : Symfonium 14.1.0 a
> été validé sur un émulateur Android 17 via une URL HTTPS publique éphémère.
>
> **Décision (user, 2026-08-03).** La validation Symfonium conditionnait le
> **tag `v2.0-beta`**, sans bloquer M4. Cette condition est maintenant satisfaite ;
> le tag reste toutefois une action explicite séparée.

## Où on en est

- **M0, M1, M2 : fermés.**
- **M3 : fermé**, y compris la matrice des quatre clients réels.

### Validation déjà passée (locale)

- `cargo fmt` : OK
- `cargo clippy` : **0 warning**
- Compilation **debug + release** : OK
- **21 tests** : OK
- Docker : `/health`, `/ready`, SQLite : OK — image **`waveflow-server:2.0.0-beta.0`** reconstruite et démarrée avec succès
- Sauvegarde SQLite + clé d'instance cohérente : OK
- Tests d'isolation par bibliothèque, anti-énumération inter-utilisateurs, administration, restauration : OK

### Réalisé notamment

- Scanner déterministe : extraction parallèle, écritures SQLite sérialisées + par lots (sur les extracteurs `waveflow-core`).
- Façade Subsonic complète demandée : XML/JSON, GET + form POST, trois méthodes d'authentification (`u/p`, `u/t/s`, `apiKey`).
- Streaming natif/transcodé, HTTP Range, cache, partages publics.
- Correctifs issus des tests navigateur + clients réels : **CORS**, **pagination de recherche**, **compatibilité DSub**, **format JSON legacy `getAlbumList`**. Contrats d'administration conformes aux spécifications OpenSubsonic.

### Matrice clients Subsonic

| Client | État |
|---|---|
| Feishin | ✅ validé |
| Substreamer | ✅ validé |
| DSub | ✅ validé |
| **Symfonium 14.1.0** | ✅ validé sur Android 17 |

## Validation Symfonium du 2026-08-09

L'instance jetable contenait un album, un artiste et deux pistes. Elle a été
exposée par un tunnel `cloudflared` aléatoire, sans donnée personnelle. Le
parcours réel a validé :

- authentification compte puis API key ;
- synchronisation complète, recherche et affichage de l'album/artiste ;
- lecture native MP3 et FLAC avec HTTP Range ;
- transcodage Opus à 64 kbit/s demandé par `maxBitRate=64` ;
- favori d'album et scrobbles relus côté serveur ;
- création puis mise à jour d'une playlist de deux pistes, relue par
  `getPlaylist`.

Trois écarts ont été découverts et couverts par des tests de non-régression :

1. après le vrai credential, Symfonium envoie un `GET ping` exact avec
   `c=Symfonium` et `test/test` pour la découverte ; seule cette enveloppe
   publique est acceptée, sans principal ni accès catalogue ;
2. `search3?query=%22%22` signifie « tout le catalogue » ;
3. `getBookmarks` fait partie du sync initial et renvoie un conteneur vide tant
   que la progression audiobook n'est pas implémentée.

### Session 2026-08-03 — tunnel monté, test non réalisé

Le tunnel HTTPS a été monté et **validé de bout en bout** (`cloudflared`,
certificat reconnu, `ping` + `getArtists` corrects depuis l'extérieur). Le test
n'a pas pu être déroulé : **Symfonium est une application Android uniquement**,
et le user teste depuis un iPhone. Le point de blocage n'est donc pas le
certificat — il est résolu — mais l'accès à un Android exécutant l'app.

Deux voies restent ouvertes pour clore la porte :

1. **Appareil Android physique** avec Symfonium installé — le plus direct.
2. **Émulateur Android + tunnel HTTPS.** La note « Symfonium ne fait pas
   confiance au CA utilisateur de l'émulateur » de `subsonic-compatibility.md`
   ne bloque plus : le tunnel fournit un certificat publiquement reconnu, ce que
   l'émulateur accepte sans CA custom. Il faut en revanche une image système
   **`google_apis_playstore`** (l'AVD `waveflow_test` local est une image
   `default`, sans Play Store) et une licence ou un essai Symfonium, l'app étant
   payante et distribuée uniquement via le Play Store.

### Correctif issu de cette session

`getOpenSubsonicExtensions` sérialisait sa liste vide en `{}` en JSON. La
spécification OpenSubsonic type ce champ comme un **tableau**, et le contrat gelé
du RFC-002 impose « JSON collection fields are arrays ». Un client à typage
strict — Symfonium est en Kotlin — peut échouer dès le handshake sur un objet
vide là où il attend une liste ; Feishin (TypeScript, tolérant) ne pouvait pas
révéler l'écart. Corrigé dans `src/subsonic.rs` (`json_array_node`), XML
inchangé, décision « liste vide » du RFC préservée, test de non-régression dans
`tests/v2_foundations.rs`.

### Procédure rejouable pour le test Symfonium (avec garde-fous)

1. Créer une **bibliothèque de test jetable** + un **compte admin/user de test**
   à **identifiants aléatoires forts** (via la CLI de bootstrap). Aucune donnée
   personnelle.
2. Exposer l'instance par un **tunnel HTTPS éphémère à URL aléatoire** (cert
   valide, pas de DNS permanent), p. ex. :
   - `cloudflared tunnel --url http://localhost:<port>` → `https://<random>.trycloudflare.com`
   - ou `ngrok http <port>`
3. S'assurer que **auth + rate-limiting** sont actifs (déjà implémentés/testés)
   et que **`u/p/t/s/apiKey` + jetons restent masqués dans les logs** (vérifié M0).
4. Connecter Symfonium à l'URL du tunnel, dérouler le parcours (indexes /
   artistes / albums / stream / star / playlists).
5. **Détruire le tunnel juste après** le test et **désactiver le compte de test**.

### Release

La porte M3 est verte. Publier / confirmer la release **`v2.0-beta`** reste une
action séparée : **jamais de tag ni de release sans demande explicite du user**.

M4 n'est plus une suite à planifier ici : il est livré (API native `/api/v2`,
PKCE, SPA embarquée, retrait de `/api/v1`). Voir `docs/M4-handoff.md`.

## Référence

Plan d'exécution complet M0→M6 : voir `docs/rfcs/` (RFC v2) et les invariants
(UUID publics, dates ms Unix, filtrage repository par user+bibliothèque, audio
read-only, catalogue = autorité serveur, clients = user-data only).
