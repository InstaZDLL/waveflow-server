# M4 — convergence native, client embarqué, PKCE : état & handoff

> Note de suivi mise à jour le 2026-08-09. **M4 est fusionné** (`14aec76`). La
> validation réelle Symfonium est terminée et ferme M3. Ce document décrit ce
> qui est fait, les décisions non évidentes à ne pas défaire, et ce qui reste.
> Aucun tag de release ne doit être créé sans demande explicite.

## Où on en est

- **M0, M1, M2 : fermés.**
- **M3 : fermé.** Symfonium 14.1.0 a validé authentification, synchronisation,
  lecture native/transcodée, favoris, scrobbles et playlists.
- **M4 : fusionné** (`14aec76`). Le nettoyage de `web/` reste porté par la PR
  #84 tant qu'elle n'est pas fusionnée.
- **M5, M6 : non commencés.**

## Validation sur bibliothèque réelle (2026-08-09)

Un scan en lecture seule de 28,8 Go a indexé 2 859 fichiers sur 2 859 sans
erreur : 1 591 AAC/M4A, 942 MP3, 285 FLAC et 41 WAV. Le catalogue obtenu compte
1 398 albums, 1 142 artistes, 27 genres, 1 368 artworks dédupliqués et 2 859
lignes FTS5. Les lectures `Range` natives des quatre codecs et un transcodage
FLAC vers Opus 64 kbit/s ont réussi.

Ce corpus a révélé deux défauts M4 corrigés après fusion : les jetons natifs
`wfapi_` créés par la CLI n'étaient pas consultés par l'authentification
`/api/v2`, et la liste des pistes était limitée silencieusement à 500 sans
pagination. Les jetons hachés honorent désormais révocation, expiration et
désactivation du compte ; la route des pistes accepte `offset`/`limit` avec un
plafond de 500. Le filtre de logs par défaut masque aussi les milliers
d'avertissements identiques produits par les atomes MP4 optionnels vides, tout
en restant surchargeable via `RUST_LOG`.

## Ce que M4 a livré

**API native `/api/v2`.** Auparavant réduite à l'authentification, aux scans et
à une liste de pistes par bibliothèque : toute vue catalogue devait passer par
la façade Subsonic.

```text
GET  /api/v2/albums · /albums/{id} · /artists · /artists/{id} · /search
GET  /api/v2/playlists · POST · PATCH · DELETE
GET  /api/v2/favorites · PUT|DELETE /favorites/{kind}/{id}
PUT  /api/v2/ratings/{kind}/{id} · POST /scrobbles · GET /now-playing
GET|PUT /api/v2/queue
POST /api/v2/oauth/authorize · POST /api/v2/oauth/token
POST /api/v2/tracks/{id}/stream-ticket · GET /api/v2/stream/{ticket}
```

**Client web embarqué** (`webapp/`, Vite + React + TanStack Router), compilé
dans le binaire par `rust_embed`. Connexion, albums, artistes, recherche,
favoris, lecture, écran de consentement OAuth.

**Retrait de la v1** : `src/api/*`, `db.rs`, `apply.rs`, `sync.rs`, les
migrations PostgreSQL et 21 fichiers de tests. Tout était déjà mort (non déclaré
dans `lib.rs` / `Cargo.toml`).

## Décisions à ne pas défaire

Ces choix ont été pris délibérément ; les « corriger » sans contexte serait une
régression.

- **Symfonium conditionne le tag `v2.0-beta`, et lui seul.** La dérogation
  approuvée par le user le 2026-08-03 couvre **le démarrage et la fusion de
  M4** : le travail M4 avance et se fusionne sans attendre M3. Ce qui reste
  bloqué, c'est le tag `v2.0-beta`, rien d'autre. C'est une dérogation
  explicite à la règle « pas de milestone suivant avant la porte précédente »
  d'`AGENTS.md`, consignée en tête de `docs/M3-symfonium-validation.md`.
  Quatre revues automatisées ont demandé de l'annuler ; c'est à écarter en s'y
  référant.
- **Un code d'autorisation est dépensé à la première présentation**, quelle que
  soit l'issue. Un verifier erroné brûle le code. Cela ferme le devinage et suit
  la règle de révocation-sur-réutilisation d'OAuth 2.1 ; un client qui rate son
  échange relance le flux.
- **Le TTL des tickets de flux est d'une heure**, pas quelques secondes. Le
  navigateur réutilise la même URL pour chaque requête `Range` : un déplacement
  tardif redemande le ticket d'origine. C'est la revérification d'accès à chaque
  utilisation qui borne le risque, pas la durée.
- **Pas de registre de clients OAuth.** Modèle RFC 8252 pour clients publics :
  PKCE + restriction des redirections (loopback / https / schéma en domaine
  inversé) remplacent l'enregistrement.
- **Le contrat Subsonic est gelé.** `search3` filtre encore en mémoire sans
  utiliser l'index FTS5 — dette assumée : y toucher risquerait une régression
  sur les trois clients validés.
- **`web/` reste dans l'arbre**, hors de toute CI, en attendant qu'on décide
  quoi en récupérer. Il n'est ni construit, ni servi, ni testé.

## Pièges connus

- **Les tests exigent `ffmpeg` et `ffprobe` sur le `PATH`.** `test_app()`
  démarre un vrai `MediaService` qui refuse de se lancer sans eux ; leur absence
  fait échouer 18 tests sur 19 d'un coup. La CI ne les installait pas, d'où un
  `main` rouge du 2026-08-02 au 2026-08-07.
- **Ordre de build : `webapp` puis `cargo`.** `rust_embed` capture
  `webapp/dist` à la compilation. `bun run build` à la racine est séquentiel
  pour cette raison. `cargo build` seul fonctionne : `build.rs` met en scène un
  placeholder sous `OUT_DIR` (jamais dans l'arbre source).
- **SQLite trie les `NULL` en premier.** Deux bugs déjà corrigés ainsi (pistes
  sans numéro, albums sans année) : penser à `NULLS LAST` sur tout tri où une
  métadonnée absente passerait devant.
- **Les migrations sont immuables une fois fusionnées** — mais celles de la PR
  #83 ne le sont pas encore tant qu'elle n'est pas mergée.
- **Toutes les tables utilisent `STRICT`.** En SQLite non-STRICT, une colonne
  `PRIMARY KEY` accepte encore `NULL` : déclarer `NOT NULL` explicitement.

## Ce qui reste

1. **Fusionner la PR #83** après relecture.
2. **Clore M3** : valider Symfonium. Le certificat n'est plus l'obstacle — un
   tunnel HTTPS à certificat reconnu a été vérifié de bout en bout le
   2026-08-03. Il manque un **Android** exécutant l'application (appareil
   physique, ou image d'émulateur `google_apis_playstore` + licence Symfonium).
   Procédure détaillée dans `docs/M3-symfonium-validation.md`.
3. **Taguer `v2.0-beta`** — ⚠️ jamais sans demande explicite du user.
4. **Trancher le sort de `web/`** (notamment le package design-tokens).
5. **M5** : réconciliation locale/serveur conservatrice.
6. **M6** : finition web studio-nocturne, bilingue, WCAG AA, Playwright.

## Dettes identifiées, non traitées

- `search3` n'exploite pas FTS5 (voir ci-dessus).
- `webapp/` n'a pas de linter. Les tests se limitent aux gardes de redirection
  (`isAllowedRedirect`, `safeInternalPath`) : aucun test de composant ni de
  parcours. La CI web installe, construit et lance ce vitest.
- Les jetons de session vivent en `localStorage`, donc exposés à une XSS. C'est
  le compromis SPA habituel ; un cookie éviterait cela mais ajouterait une
  authentification ambiante et une surface CSRF à une API sinon purement par
  en-tête. **Porte de sortie : trancher avant le tag `v2.0` stable** (pas avant
  la beta) — soit adopter un cookie `httpOnly` + protection CSRF, soit acter le
  risque par écrit dans ce document avec la justification retenue. Ne pas taguer
  la stable tant que l'une des deux branches n'est pas tranchée.
