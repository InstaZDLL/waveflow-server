# M3 — Subsonic/OpenSubsonic : état & handoff

> Note de suivi (2026-08-03). La porte M3 reste **ouverte** : un seul point
> manque, la validation **Symfonium**, qui exige une URL HTTPS publique à
> certificat reconnu. Ce document décrit ce qui est fait et la procédure exacte
> pour clore M3, à reprendre par le prochain agent.
>
> **Décision (user, 2026-08-03).** La validation Symfonium conditionne désormais
> le **tag `v2.0-beta`**, plus le **démarrage de M4**. M4 démarre avec la porte
> M3 partiellement ouverte : 3 clients réels sur 4 sont validés, et le contrat
> Subsonic est gelé au RFC-002. Aucune release ne peut être coupée tant que
> Symfonium n'est pas passé.

## Où on en est

- **M0, M1, M2 : fermés.**
- **M3 : implémenté et validé localement**, porte **ouverte** (Symfonium seul).

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
| **Symfonium** | ⛔ **bloqué** — exige une URL **HTTPS publique à certificat reconnu**, et l'app n'existe que sur **Android** |

## Ce qui reste pour fermer M3

Valider **Symfonium** contre l'instance exposée en HTTPS public, puis marquer la
porte M3 verte. Ce n'est plus un préalable au démarrage de M4 (voir la note de
suivi en tête), mais cela reste un préalable au tag `v2.0-beta`.

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

### Procédure recommandée pour le test Symfonium (avec garde-fous)

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

### Après Symfonium OK

- Marquer la **porte M3** verte.
- Publier / confirmer la release **`v2.0-beta`** (⚠️ **jamais cut de release sans
  demande explicite du user**).
- Démarrer **M4** : API native `/api/v2`, WaveFlow Desktop (Authorization Code +
  PKCE, catalogue serveur = source distante séparée, pas de fusion avec la biblio
  locale), SPA React/TanStack embarquée dans le binaire, retrait de `/api/v1` au
  passage v2.0 stable.

## Référence

Plan d'exécution complet M0→M6 : voir `docs/rfcs/` (RFC v2) et les invariants
(UUID publics, dates ms Unix, filtrage repository par user+bibliothèque, audio
read-only, catalogue = autorité serveur, clients = user-data only).
