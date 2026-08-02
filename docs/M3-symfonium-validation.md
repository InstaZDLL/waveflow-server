# M3 — Subsonic/OpenSubsonic : état & handoff

> Note de suivi (2026-08-02). La porte M3 reste **ouverte** : un seul point
> manque, la validation **Symfonium**, qui exige une URL HTTPS publique à
> certificat reconnu. Ce document décrit ce qui est fait et la procédure exacte
> pour clore M3, à reprendre par le prochain agent.

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
| **Symfonium** | ⛔ **bloqué** — exige une URL **HTTPS publique à certificat reconnu** |

## Ce qui reste pour fermer M3

Valider **Symfonium** contre l'instance exposée en HTTPS public, puis marquer la
porte M3 verte et démarrer **M4**.

### Décision (user, 2026-08-02)

Le test Symfonium via tunnel HTTPS **n'a pas été fait dans la session courante**.
Il sera réalisé **avec un autre agent**. Ce document sert de reprise.

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
