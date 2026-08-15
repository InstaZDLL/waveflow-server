# M4 — convergence native, client embarqué, PKCE : état & handoff

> Note de suivi mise à jour le 2026-08-14. **M4 est livré côté serveur mais
> reste ouvert** : le socle est fusionné en `14aec76`, son complément en
> `6716df9` (PR #94), et la validation Symfonium a fermé M3. Ce qui manque est
> hors de ce dépôt — l'intégration WaveFlow Desktop n'a pas tourné de bout en
> bout, et c'est elle qui ferme la porte. Voir
> [`desktop-v2-integration-gap.md`](desktop-v2-integration-gap.md).
>
> Ce document décrit ce qui est fait, les décisions non évidentes à ne pas
> défaire, et ce qui reste. Aucun tag de release ne doit être créé sans demande
> explicite.

## Où on en est

- **M0, M1, M2 : fermés.**
- **M3 : fermé.** Symfonium 14.1.0 a validé authentification, synchronisation,
  lecture native/transcodée, favoris, scrobbles et playlists.
- **M4 : ouvert.** Tout le travail serveur est livré — socle en `14aec76`,
  complément en `6716df9`, `web/` retiré par la PR #84 après extraction de ses
  design tokens. La porte attend la validation de l'intégration Desktop, plus
  la revalidation Subsonic due au passage à FTS5 et aux extensions annoncées.
- **M5, M6 : non commencés.**

`main` est vert sur les deux runners et aucune PR n'est ouverte au moment de
cette note.

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
GET  /api/v2/tracks/{id}
GET  /api/v2/playlists · POST · PATCH · DELETE
GET  /api/v2/favorites · PUT|DELETE /favorites/{kind}/{id}
PUT  /api/v2/ratings/{kind}/{id} · POST /scrobbles · GET /now-playing
GET|PUT /api/v2/queue
GET|POST /api/v2/shares · PATCH|DELETE /api/v2/shares/{share_id}
GET|POST /api/v2/libraries
PUT|DELETE /api/v2/libraries/{library_id}/members/{user_id}
GET|POST /api/v2/admin/users · PATCH|DELETE /api/v2/admin/users/{username}
PUT|DELETE /api/v2/admin/users/{username}/subsonic-credential
GET /api/v2/sync/snapshot · /changes · WS /sync/socket · PUT /sync/ack
POST /api/v2/oauth/authorize · POST /api/v2/oauth/token
GET /api/v2/tracks/{track_id}/stream
POST /api/v2/tracks/{track_id}/stream-ticket · GET /api/v2/stream/{ticket}
```

**Client web embarqué** (`webapp/`, Vite + React + TanStack Router), compilé
dans le binaire par `rust_embed`. Connexion, albums, artistes, recherche,
favoris, playlists, file d'attente persistante, partages, lecture, écran de
consentement OAuth et administration des bibliothèques, scans, comptes et
identifiants Subsonic. Le lecteur n'est monté qu'après authentification afin de
charger la file du bon compte à chaque nouvelle session.

**Retrait de la v1** : `src/api/*`, `db.rs`, `apply.rs`, `sync.rs`, les
migrations PostgreSQL et 21 fichiers de tests. Tout était déjà mort (non déclaré
dans `lib.rs` / `Cargo.toml`).

**Synchronisation des données utilisateur** (`src/sync.rs`), spécifiée par
[RFC-003](rfcs/RFC-003-waveflow-sync-v2.md). Le serveur reste l'autorité du
catalogue ; le protocole ne synchronise que l'état possédé par le compte —
playlists, favoris, notes, historique, file d'attente et partages. Il n'importe
jamais une piste serveur dans le catalogue local et ne devine jamais une
correspondance locale/serveur : cette réconciliation est M5 et exige son propre
RFC. REST est la source durable, le WebSocket n'est qu'une notification qu'un
curseur plus récent existe peut-être. Les mutations portent un
`X-WaveFlow-Operation-Id` qui rend le rejeu sûr, et un `X-WaveFlow-Device-Id`
que le serveur refuse s'il appartient à un autre compte.

## Décisions à ne pas défaire

Ces choix ont été pris délibérément ; les « corriger » sans contexte serait une
régression.

- **La validation Symfonium a fermé M3 après la fusion de M4.** La dérogation
  approuvée le 2026-08-03 autorisait explicitement le démarrage et la fusion de
  M4 avant cette validation, sans autoriser le tag `v2.0-beta`. Elle expliquait
  l'écart temporaire avec l'ordre des jalons d'`AGENTS.md` ; elle est désormais
  historique puisque la porte M3 est fermée.
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
- **Le contrat Subsonic est gelé**, à une exception documentée : `search3`
  s'appuie désormais sur FTS5 (voir « Recherche Subsonic » plus bas). Toute
  autre évolution du comportement observable reste proscrite.
- **`web/` a été supprimé** (décision user du 2026-08-08) après extraction de
  `packages/design-tokens` vers `webapp/src/design-tokens/`. Le reste était
  arrimé à la hiérarchie profil/bibliothèque de la v1 et aux server functions
  Better Auth : git en garde l'historique.

## Pièges connus

- **Les tests exigent `ffmpeg` et `ffprobe` sur le `PATH`.** `test_app()`
  démarre un vrai `MediaService` qui refuse de se lancer sans eux ; leur absence
  fait échouer la quasi-totalité de la suite d'un coup. La CI ne les installait
  pas, d'où un `main` rouge du 2026-08-02 au 2026-08-07. Une seconde variante a
  frappé le 2026-08-09 : `choco install` sort avec le code 0 quand le dépôt
  Chocolatey répond 503, donc l'étape Windows était verte en n'installant rien
  et l'échec ne se voyait que cinq minutes plus tard. Le job réessaie
  désormais, puis exige que `ffmpeg -version` et `ffprobe -version` répondent.
  **Devant un échec massif de la suite, vérifier d'abord la présence de FFmpeg
  avant de chercher une régression.**
- **Ordre de build : `webapp` puis `cargo`.** `rust_embed` capture
  `webapp/dist` à la compilation. `bun run build` à la racine est séquentiel
  pour cette raison. `cargo build` seul fonctionne : `build.rs` met en scène un
  placeholder sous `OUT_DIR` (jamais dans l'arbre source).
- **SQLite trie les `NULL` en premier.** Deux bugs déjà corrigés ainsi (pistes
  sans numéro, albums sans année) : penser à `NULLS LAST` sur tout tri où une
  métadonnée absente passerait devant.
- **Les migrations sont immuables une fois fusionnées.** Celles de M4 sont
  désormais sur `main` ; toute évolution ajoute une nouvelle migration datée.
- **Toutes les tables utilisent `STRICT`.** En SQLite non-STRICT, une colonne
  `PRIMARY KEY` accepte encore `NULL` : déclarer `NOT NULL` explicitement.
- **La visibilité se filtre dans la requête, jamais après.** `ratings_on` et
  `starred_ids_on` portent chacun leurs prédicats `EXISTS` contre
  `library_member`. Toute nouvelle projection de données utilisateur doit faire
  de même : l'oubli ne casse aucun test fonctionnel et ne se voit qu'à la
  relecture.
- **Le writer gate est un verrou d'écriture global au processus.** L'attendre
  peut prendre du temps derrière un scan. Ne rien lire de coûteux en le tenant
  (`drop(_writer)` juste après `tx.commit()`), et considérer qu'un état lu avant
  de l'acquérir a pu changer depuis.
- **`@vitejs/plugin-react` 6 exige Vite ≥ 7** — il importe `vite/internal`, qui
  n'existe pas avant. Les bumps front se testent ensemble : pris isolément,
  celui du plugin échoue au build.

## Ce qui reste

1. **Valider l'intégration WaveFlow Desktop** contre `/api/v2` et Authorization
   Code + PKCE : catalogue distant, streaming, playlists, favoris, notes,
   historique, file d'attente et partages, plus la reconnexion, la
   rotation/révocation des refresh tokens et la reprise de synchronisation.
   C'est le vrai test de convergence : la bibliothèque réelle avait déjà révélé
   deux défauts qu'aucun test ne voyait. Le portage est en cours dans le dépôt
   Desktop ; il n'a pas encore tourné de bout en bout, et le retrait de son
   protocole v1 attend cette exécution. **Le Desktop lit avec un `Authorization:
   Bearer` sur `/api/v2/tracks/{id}/stream` — les tickets scellés restent
   réservés à `<audio src>`, qui ne peut pas porter d'en-tête.**
2. **Taguer une release uniquement sur demande explicite du user.** M3 et sa
   validation Symfonium sont terminés ; aucune action de compatibilité ne reste
   ouverte pour cette porte.
3. **M5** : réconciliation locale/serveur conservatrice. **Commencer par un
   RFC** — liaison automatique sur hash complet unique seulement, MBID en
   suggestion à confirmer, aucun rapprochement flou par titre/artiste/durée.
4. **M6** : finition web studio-nocturne, bilingue, WCAG AA, Playwright.

## Outillage front

`webapp/` utilise **Biome** (lint + format en une passe) plutôt qu'eslint +
prettier : `bun run lint`, `bun run format`. La suite vitest tourne sous jsdom,
nécessaire aux design tokens qui écrivent sur `document.documentElement`. La CI
web lint, construit et teste. La chaîne est sur Vite 8, TypeScript 7 et React 19.

`webapp/src/vite-env.d.ts` déclare `/// <reference types="vite/client" />`. Ce
fichier n'est pas décoratif : sans lui, TypeScript 7 rejette l'import
side-effect de `./styles.css` (TS2882). TypeScript 5 le tolérait.

La directive `biome-ignore` de `useAsync` (`src/pages.tsx`) est placée **juste
avant `}, deps)`**, pas avant `useEffect` : la règle se déclenche sur
l'argument de dépendances, et déplacée plus haut elle ne supprime plus rien —
vérifié.

## Dettes identifiées, non traitées

- `webapp/` n'a pas de test de **rendu** ni de parcours navigateur. La suite
  couvre désormais les gardes de redirection, les design tokens, la pagination
  et le rafraîchissement de session — mais pas les composants. Un smoke test
  manuel sur installation vide valide setup, session, rôles, playlists, favoris,
  queue, partages, bibliothèques, scans, comptes et rotation d'identifiant
  Subsonic. La CI web lint (Biome), construit et lance Vitest.

## Dettes fermées, à ne pas rouvrir par erreur

- **La session navigateur ne dépend plus de `localStorage`** : access token
  court en mémoire, refresh rotatif dans un cookie HttpOnly/SameSite, contrôle
  d'origine et double-submit CSRF sur refresh/logout. Les deux `removeItem`
  restants dans `webapp/src/api.ts` ne font que purger les entrées héritées de
  la première version de M4 ; ce ne sont pas des lectures de session. Cette
  dette conditionnait le tag `v2.0` stable : elle est levée.
- **Le DCO n'exige plus de dérogation pour Dependabot.** Le workflow accepte
  l'adresse de signature réellement émise par le bot, différente de celle sous
  laquelle il committe. Les sept bumps du 2026-08-09 sont passés sans override
  du contrôle DCO.
- **`.github/CODEOWNERS` existe.** Le ruleset « Main » exige une revue de code
  owner ; sans ce fichier, aucun owner n'existait, la condition était donc
  insatisfaisable et **chaque** fusion passait par un override propriétaire. Ne
  pas le supprimer en croyant simplifier : ce serait remettre l'override
  systématique, c'est-à-dire une protection qui ne protège plus.

## Amorcer une instance de test

`scripts/seed-dev-instance.sh [data-dir]` monte une instance jetable : il génère
l'audio avec FFmpeg, crée l'admin, enregistre et scanne la bibliothèque, puis
imprime identifiants et jeton `wfapi_`. Six pistes, trois albums, trois artistes,
dont un crédit multi-artistes et une piste sans numéro pour exercer le tri
`NULLS LAST`.

Il existe parce qu'un catalogue vide ne permet d'exercer que les playlists :
favoris, notes, scrobbles et file d'attente ont tous besoin d'un identifiant de
piste réel. Deux pièges relevés en s'en servant : `POST /api/v2/auth/login` exige
`device_name` dans le corps, et la projection `/libraries/{id}/tracks` n'expose
ni `track_number`, ni `disc_number`, ni `year` — ils sont dans `/albums/{id}`.

## Contrat d'API affiné pour le client natif (2026-08-12)

Trois décisions prises à la demande de l'agent Desktop, toutes sur `/api/v2`
uniquement — la façade Subsonic est inchangée et ses tests n'ont pas bougé.

- **409 pour les conflits.** Un `operation_id` rejoué avec une charge différente
  répond `409` / `code: "conflict"` au lieu de `422`. Les collisions de nom de
  compte et un `setup` répété aussi. La façade Subsonic répondait déjà 409 : ce
  changement aligne l'API native sur elle.
- **Effacer un champ optionnel.** `PATCH` accepte `clear`, une liste nommant les
  champs à vider : `comment` pour une playlist, `description` et `expires_at`
  pour un partage. Un nom inconnu est **refusé** (422) et non ignoré, pour qu'un
  `expiresAt` en camelCase ne passe pas pour un succès. L'effacement fait partie
  de l'empreinte d'opération : poser une expiration et la retirer sont deux
  mutations distinctes et ne peuvent pas partager un identifiant de rejeu.
- **`cursor_expired`.** `/sync/changes` refuse un curseur antérieur au plus
  ancien événement conservé, avec `409` et `code: "cursor_expired"`. Le statut
  est partagé avec les conflits : **c'est le code qui commande la réaction**,
  pas le statut. La reprise est un **snapshot complet**, jamais une reprise
  depuis le plancher survivant — celle-ci réussirait en sautant les événements
  compactés. Inatteignable tant que le journal reste append-only, mais
  implémenté et testé pour que les clients écrivent leur branche de reprise
  contre un contrat réel.

Trois ajouts demandés par le client Android, tous sur `/api/v2` :

- **`GET /api/v2/artwork/{artwork_id}`** sert les pochettes derrière le Bearer
  natif. Elle accepte un `artwork_hash` ou l'id d'une piste, d'un album ou d'un
  artiste. Sans elle, un catalogue distant s'affichait **sans aucune pochette** :
  les charges portaient `artwork_hash` et seule la façade Subsonic, avec ses
  identifiants distincts, savait le résoudre. La lecture du fichier est partagée
  avec `getCoverArt` pour que les deux surfaces ne divergent pas.
- **`full_hash` publié** sur `SongItem` et `TrackRecord` : BLAKE3 non keyed,
  hexadécimal, sur le fichier entier. C'est la clé de réconciliation de M5, que
  les clients peuvent recalculer localement. Elle empreinte le **fichier**, pas
  l'audio décodé. **L'algorithme fait partie du contrat** : en changer un jour
  signifie ajouter un champ, jamais redéfinir celui-ci.
- **`album_count` sur `/artists/{id}`**, comme sur `/artists`.

**L'URL d'un ticket de flux est toujours relative**, jamais préfixée par
`WAVEFLOW_PUBLIC_URL` — contrairement à une URL de partage, faite pour sortir de
l'application. Une valeur absolue permettrait d'orienter la lecture vers un hôte
auquel l'utilisateur ne s'est jamais authentifié, et les clients natifs la
rejettent. Un test le vérifie avec `public_url` configurée : **ne pas
« harmoniser » les deux**.

**L'OpenAPI déclare enfin sa sécurité.** Schéma bearer exigé globalement, douze
opérations explicitement publiques (celles qui portent leur propre justificatif
ou aucun, `/api/v2/stream/{ticket}` compris), et les en-têtes
`X-WaveFlow-Operation-Id` / `-Device-Id` documentés sur les onze écritures
`user-data` — jamais sur les lectures, qui ne portent pas d'identifiant
d'opération. Sans cela, un client généré depuis le document ne s'authentifiait
nulle part et perdait la sûreté de rejeu.

## Recherche Subsonic sur FTS5 (2026-08-13)

`search3` interrogeait le catalogue entier chargé en mémoire, puis le filtrait
par sous-chaîne. Il s'appuie désormais sur l'index FTS5, comme `/api/v2/search`.
**C'est le seul écart autorisé au gel du contrat Subsonic**, et il change les
résultats — d'où ce qui suit.

- **Gagné :** l'insensibilité aux diacritiques. Le tokenizer
  `unicode61 remove_diacritics 2` fait que « echo » atteint « Écho », ce que le
  test par sous-chaîne en minuscules ne faisait pas.
- **Gagné :** la recherche ne matérialise plus tout le catalogue à chaque appel.
- **Perdu :** la correspondance en milieu de mot. « cho » ne trouve plus
  « Echo ». Le dernier terme est traité comme un préfixe (`ech*`), donc la
  frappe incrémentale — le mode d'interrogation normal d'un client — continue
  de fonctionner.

Deux invariants préservés, et testés parce qu'ils cassent en silence :

- la requête littérale `""` reste « tout le catalogue » et emprunte toujours le
  snapshot complet, FTS5 n'ayant pas d'expression signifiant « tout » ;
- un nœud `album` annonce **sa** taille, pas le nombre de pistes qui matchent.
  `album_node` dérive `songCount` et `duration` de la liste de pistes qu'on lui
  passe : lui donner les seules correspondances ferait annoncer « 2 pistes » à
  un album qui en compte 12, faussement et sans erreur.

**À revalider avec les quatre clients réels** (Symfonium, DSub, Feishin,
Substreamer) avant le tag `v2.0-beta` : c'est la surface que le gel protégeait.

## Extensions OpenSubsonic annoncées (2026-08-13)

`getOpenSubsonicExtensions` renvoyait un conteneur vide, ce qui disait à tout
client tiers que le serveur ne prend rien en charge d'optionnel. Il annonce
maintenant les trois extensions réellement implémentées et couvertes par la
suite : **`formPost`**, **`apiKeyAuthentication`** et **`transcodeOffset`**.

**N'annoncer que ce qui est implémenté.** Déclarer une extension que le serveur
n'honore pas est pire que n'en déclarer aucune : le client cesse de sonder et
se met à en dépendre.

La spécification ne définit **aucune forme XML** pour cette méthode ; `versions`
est donc un tableau JSON et se rend en `"[1]"` dans la branche XML. Les clients
qui utilisent la méthode demandent du JSON.

Groupé volontairement avec le passage à FTS5 : les deux touchent la surface
Subsonic, une seule campagne de revalidation les couvre.

## Correctifs de sécurité postérieurs à la fusion (2026-08-09)

Trois défauts trouvés en relecture après la fusion de #94, tous corrigés sur
`main` :

- `starred_ids_on` renvoyait toutes les lignes `user_star` sans filtre de
  visibilité, là où `ratings_on` restreignait déjà les siennes. Un auditeur
  retiré d'une bibliothèque continuait de voir ses favoris hors périmètre, via
  `/api/v2/favorites`, `getStarred` et le journal de synchronisation. Couvert
  par une assertion de non-régression, vérifiée comme échouant sans le
  correctif.
- `public_share` servait un partage supprimé ou expiré pendant l'attente du
  writer gate. L'`UPDATE` arbitre désormais révocation et expiration : aucune
  ligne affectée signifie 404.
- `create_subsonic_user` et `update_user` tenaient le writer gate pendant
  `self.users()`, une lecture complète des comptes.

## État des dépendances (2026-08-09)

Toutes les mises à jour en attente ont été fusionnées : `base64` 0.23, `md-5`
0.11, `chacha20poly1305` 0.11, le groupe `cargo-patch-and-minor`, puis Vite 8,
TypeScript 7 et `@vitejs/plugin-react` 6 côté front.

**`chacha20poly1305` 0.11 mérite une note.** `SecretBox` chiffre le mot de passe
Subsonic *au repos*. La compatibilité du format a été vérifiée avant fusion en
scellant un secret avec 0.10 puis en le déchiffrant avec 0.11 : le format reste
celui de la RFC 8439, nonce et ciphertext étant stockés séparément par
`src/security.rs`. Sauvegarder malgré tout `data/waveflow.db` et
`data/instance.key` **ensemble** avant le prochain déploiement — une valeur
chiffrée est irrécupérable sans sa clé.
