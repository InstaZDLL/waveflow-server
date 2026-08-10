# WaveFlow Desktop ↔ serveur v2 : inventaire de l'écart

> Relevé du 2026-08-10, établi en lecture seule sur
> [`InstaZDLL/WaveFlow`](https://github.com/InstaZDLL/WaveFlow) à `802f189b`.
> Il décrit ce que le Desktop attend aujourd'hui, ce que le serveur v2 offre, et
> ce qui n'a pas d'équivalent. Aucun code n'a été modifié dans l'un ou l'autre
> dépôt. Le contrat cible est [RFC-003](rfcs/RFC-003-waveflow-sync-v2.md).

## Constat

Le Desktop parle intégralement au serveur **v1**, qui n'existe plus. Les six
routes qu'il consomme ont zéro occurrence dans le code du serveur v2, et son
flux de connexion s'appuie sur un front web supprimé par la PR #84.

Ce n'est donc pas une adaptation de la couche de synchronisation : c'est le
remplacement d'un protocole bidirectionnel à horloges logiques par un journal
linéaire dont le serveur est l'autorité.

## Surface concernée

Environ 6 000 lignes de Rust, réparties ainsi :

| Module | Lignes | Sort attendu |
| --- | --- | --- |
| `sync/digest/{mod,client,entity_client}.rs` | 1 178 | Supprimé — remplacé par `snapshot` |
| `sync/backfill/{mod,pull,lww}.rs` | 1 349 | Supprimé — la résolution LWW n'a plus d'objet |
| `sync/ws.rs` | 557 | Réécrit — le socket devient un simple réveil |
| `sync/queue.rs` | 573 | Conservé, ré-outillé sur `X-WaveFlow-Operation-Id` |
| `sync/drain.rs` | 543 | Réécrit — plus d'endpoint d'ops générique |
| `commands/sync.rs` | 624 | Réécrit |
| `server_client.rs` | 357 | Réécrit — base URL, en-têtes, auth |
| `commands/server_auth.rs` | 227 | Réécrit — PKCE |
| `sync_stub.rs` | 368 | À réévaluer |
| `sync/{cursor,hooks,canonical,track_snapshots}.rs` | ~640 | Conservés, adaptés |
| `commands/share.rs` | 292 | Réécrit sur `/api/v2/shares` |

Plus deux fichiers TypeScript : `src/lib/tauri/serverAuth.ts` et
`src/components/views/settings/ServerAccountCard.tsx`.

## Correspondance des routes

| Desktop aujourd'hui | Serveur v2 | Nature |
| --- | --- | --- |
| `GET /api/v1/sync/ops?since=N` | `GET /api/v2/sync/changes?after=N&limit=` | Renommage + pagination explicite (`next_cursor`, `has_more`) |
| `POST /api/v1/sync/ops` | *(aucun)* | **Disparu.** Les mutations passent par les endpoints REST métier, porteurs de `X-WaveFlow-Operation-Id` |
| `POST /api/v1/sync/ack` `{device_id, last_seen_id}` | `PUT /api/v2/sync/ack` `{device_id, cursor}` | Verbe et champs changés |
| WS poussant des ops | `GET /api/v2/sync/socket?after=` → `{"cursor": N}` | **Sémantique inversée** : notification, jamais transport d'état |
| `GET /api/v1/sync/digest` | *(aucun)* | **Disparu** — `GET /api/v2/sync/snapshot` remplace la réconciliation par empreinte |
| `GET /api/v1/sync/entity` | *(aucun)* | **Disparu** — contenu porté par `snapshot` / `changes` |
| `GET /api/v1/profiles` | *(aucun)* | **Disparu** — voir « modèle de profils » |
| `POST /api/v1/share/playlists/by-canonical/{p}/{pl}` | `POST /api/v2/shares` | Modèle de partage entièrement différent |

## Ce qui devient sans objet

Ces mécanismes existent parce que la v1 acceptait des écritures concurrentes
arbitrées côté client. Le serveur v2 est autorité et son journal est
strictement ordonné : les conserver serait porter une complexité que le
protocole ne demande plus.

- **Horloges logiques.** `sync/lamport` et `sync/hlc` estampillent chaque op
  sortante pour un ordre global entre pairs. Le curseur du journal v2 fournit
  cet ordre à lui seul.
- **Réconciliation par empreinte.** `sync/digest` compare des digests par entité
  pour détecter une divergence. `GET /sync/snapshot` renvoie une représentation
  cohérente côté writer que le client remplace atomiquement.
- **Backfill LWW.** `sync/backfill/lww.rs` arbitre les conflits au dernier
  écrivain. En v2, un conflit n'est pas arbitré côté client : réutiliser un
  `operation_id` avec une empreinte différente est **rejeté en conflit**, et un
  client qui ne sait pas appliquer un événement connu jette sa projection et
  reprend un snapshot.
- **Compaction et `410 Gone`.** `catchup_pull` gère un serveur ayant compacté
  au-delà du curseur. Le journal v2.0 est append-only ; RFC-003 conditionne
  toute rétention future à un plancher de snapshot. Ce chemin peut disparaître,
  mais **le remplacer par une reprise de snapshot** plutôt que le supprimer sans
  substitut.

## Ce qui reste nécessaire

- **`device_id`.** Toujours central : le serveur refuse un appareil appartenant
  à un autre compte, et l'ACK est par appareil.
- **File d'attente locale.** L'écriture hors ligne reste nécessaire. Ce qui
  change est son contenu : non plus des « ops » génériques, mais des appels REST
  métier rejouables, chacun porteur d'un `operation_id` stable.
- **Mapping identifiants locaux ↔ serveur.** `sync/canonical.rs` et la table
  `sync_id_map` gardent leur raison d'être : les identifiants publics v2 sont des
  UUID, le Desktop travaille sur des rowid locaux.
- **Curseur, application entrante, reconnexion.** Conservés, adaptés aux
  nouvelles formes.

## Décisions actées (2026-08-10)

Ces points étaient ouverts au moment du relevé. Ils sont tranchés, et chacun a
été vérifié contre le code du serveur : aucun ne demande d'ajout côté serveur.

### 0. Le Desktop est un client universel

**Décision (user).** Le Desktop doit pouvoir se connecter à n'importe quel
serveur Subsonic — Navidrome, Airsonic, Gonic — et pas seulement à WaveFlow. Ce
choix commande tous les autres : il impose une abstraction « source distante »
avec deux implémentations, au lieu d'un client v2 unique.

Deux interfaces distinctes plutôt qu'une seule, pour que la synchronisation
reste une capacité et non le socle obligatoire du client :

```text
   MusicServer (obligatoire)            SyncProvider (optionnel)
   catalogue, recherche, lecture,       snapshot, changes, ack, socket
   user-data selon capacités
          │                                      │
   ┌──────┴───────┐                              │
Subsonic      WaveFlow  ────────────────────────▶┘
(Navidrome,   (implémente MusicServer
 Airsonic…)    + SyncProvider)
```

Le dénominateur commun est large : la façade Subsonic couvre **toutes** les
mutations dont le Desktop a besoin — `star`/`unstar`, `setRating`, les playlists
en CRUD complet, `scrobble`, `savePlayQueue`/`getPlayQueue` et les partages.

**Détection.** Toute réponse Subsonic de WaveFlow porte `type="waveflow"`,
`serverVersion` et `openSubsonic="true"` (`src/subsonic.rs`). Un `ping` suffit
donc à décider si `SyncProvider` peut être activé.

> **Piège vérifié.** Ne pas détecter les capacités de WaveFlow par
> `getOpenSubsonicExtensions` : cette méthode renvoie aujourd'hui un conteneur
> **vide**. Un client qui s'y fierait conclurait que WaveFlow n'offre aucune
> extension, alors qu'il offre l'intégralité de l'API v2. Le discriminant est
> `type`, pas la liste d'extensions.

**Ce que seul WaveFlow offre**, et qui doit donc être traité comme une capacité
optionnelle et non comme un prérequis : le journal (`/sync/snapshot`,
`/sync/changes`), l'ACK par appareil, le WebSocket de réveil, et surtout
l'idempotence des mutations.

**Modèle d'identifiants.** Les identifiants distants sont des **chaînes
opaques**, jamais des UUID typés. WaveFlow sérialise ses UUID
(`id.to_string()`), ce qui rend ses deux surfaces interchangeables sans table de
correspondance — mais Navidrome et consorts émettent des identifiants textuels
d'une autre forme. Le Desktop doit donc :

- traiter tout identifiant distant comme `String`, sans le parser en `Uuid` ;
- indexer localement sur la clé composée `(profile_id, remote_id)`, puisque deux
  serveurs peuvent émettre le même identifiant sans rapport ;
- garder le cache de catalogue distinct de l'état synchronisé, l'un étant
  reconstructible et l'autre non ;
- ne jamais supposer qu'une capacité est présente sans l'avoir constatée.

**Conséquence sur l'idempotence.** `src/subsonic.rs` ne lit jamais
`X-WaveFlow-Operation-Id` : ces en-têtes n'existent que sur les routes v2. Une
mutation Subsonic n'est donc **pas rejouable sans risque**. Si la réponse se
perd, le client ignore si le serveur a appliqué : rejouer duplique le scrobble
ou la playlist. Les deux implémentations n'offrent pas la même garantie hors
ligne, et la file d'attente doit le savoir :

- `WaveflowSource` — rejeu sûr par `operation_id` ; la file peut réémettre
  librement.
- `SubsonicSource` — pas de rejeu aveugle. Après une réponse perdue, relire
  l'état avant de décider, ou accepter le doublon pour les entités idempotentes
  par nature (`star`, `setRating`) et s'abstenir pour celles qui ne le sont pas
  (`scrobble`, `createPlaylist`).

### 1. Un profil Desktop = un compte serveur

Le profil reste l'unité **locale** qui sépare comptes, jetons, mappings et
curseurs. Son identité de synchronisation devient l'`account_id` fourni par le
serveur ; `profile_canonical_id` cesse d'exister dans le protocole.

```text
Desktop                          Serveur
└── Profile                      └── Account
    ├── server_url                   ├── Device A
    ├── remote_identity ────────────▶├── Device B
    ├── auth (tokens | u/p)          ├── Library 1
    ├── sync state (si WaveFlow)     └── Library 2
    └── active_library_id (option)
```

L'identité distante est **polymorphe**, puisque tous les serveurs n'ont pas le
même modèle de compte :

```rust
enum RemoteIdentity {
    Waveflow { account_id: Uuid, device_id: Uuid, cursor: u64 },
    Subsonic { username: String },
}
```

Un serveur Subsonic tiers n'a ni identifiant de compte en UUID, ni notion
d'appareil, ni curseur : ces trois champs n'existent que dans la branche
WaveFlow. Les câbler dans la structure commune obligerait à les rendre
optionnels partout et à répandre des `unwrap` sur des cas qui ne peuvent pas
survenir.

Un profil par bibliothèque a été écarté : une bibliothèque est une ressource de
contenu, un profil une identité et une session. Un compte donnant accès à
plusieurs bibliothèques produirait sinon des profils artificiels pour un même
utilisateur.

**Rien à ajouter côté serveur.** `POST /api/v2/auth/login` renvoie déjà l'`id`
du compte et le `device_id` (`WebAuthResponse`), et le serveur déduit
l'utilisateur du Bearer seul. `active_library_id` correspond au paramètre
`library_id: Option<Uuid>` que les projections catalogue acceptent déjà
(`list_albums`, `list_artists`) : c'est un filtre existant, pas une notion à
introduire.

### 2. Authorization Code + PKCE en loopback

Le flux actuel ouvre `<web-url>/desktop-login?cb=…` et attend un JWT Better Auth
en query string ; la cible est PKCE (S256) sur `/api/v2/oauth/authorize` et
`/api/v2/oauth/token`. Le Desktop possède déjà un listener loopback et un
générateur d'aléa (chemin Spotify) : la mécanique existe, le protocole change.

Attention : un code v2 est **dépensé à la première présentation**, quelle qu'en
soit l'issue. Un verifier erroné brûle le code et impose de relancer le flux ;
ce n'est pas un bug à contourner par une nouvelle tentative sur le même code.

PKCE ne vaut que pour la branche WaveFlow. Un serveur Subsonic tiers
s'authentifie par `u/p`, `u/t/s` ou `apiKey` selon ce qu'il accepte : c'est une
seconde forme d'authentification à porter, pas une dégradation de la première.

### 3. Bearer direct pour la lecture, pas de ticket

Le Desktop lit par `GET /api/v2/tracks/{track_id}/stream` avec
`Authorization: Bearer`. Vérifié dans `src/media.rs` : ce handler exige le
Bearer et accepte `format`, `bitrate` et `offset_ms`, avec 206/416 sur les
`Range`. Les tickets scellés restent réservés aux consommateurs qui ne peuvent
pas porter d'en-tête — `<audio src>` dans le navigateur.

Contre un serveur tiers, la lecture passe par `/rest/stream` avec les
identifiants Subsonic. `MusicServer` expose donc une URL de flux (et l'en-tête
éventuel à joindre), pas une route en dur.

## Contraintes de conception de la file d'attente

La file locale ne contient plus des ops abstraites mais des mutations métier
rejouables. Une forme typée est préférable à des requêtes HTTP sérialisées :

```rust
enum Mutation {
    AddFavorite { operation_id: Uuid, track_id: Uuid },
    SetRating   { operation_id: Uuid, track_id: Uuid, rating: u8 },
    UpdatePlaylist { operation_id: Uuid, playlist_id: Uuid, /* … */ },
}
```

Trois propriétés du serveur contraignent cette file, et les ignorer produirait
des échecs dans des cas parfaitement ordinaires :

- **Une entrée enfilée est immuable.** Le serveur stocke une empreinte canonique
  de l'action, de la ressource visée et du payload normalisé. Réutiliser un
  `operation_id` avec une empreinte différente est **rejeté en conflit**, pas
  traité comme un rejeu :

  ```rust
  if record.intent_hash.as_deref() != Some(intent.as_bytes()) {
      return Err(SyncError::Conflict);
  }
  ```

  Donc si l'utilisateur corrige son geste avant que la file ne se vide — renommer
  une playlist déjà enfilée, par exemple — il faut émettre une seconde mutation
  avec un **nouvel** `operation_id`, ou fusionner les deux avant enfilement en
  régénérant l'identifiant. Muter une entrée en attente en gardant son
  `operation_id` casse.

- **Un rejeu de partage restitue la même URL.** `derive_share_token(id)` est
  déterministe (BLAKE3 keyed sur la clé d'instance et l'UUID). La création de
  partage se rejoue donc comme n'importe quelle mutation, sans traitement
  particulier et sans forger une seconde URL.

- **L'ACK n'est pas un prérequis.** RFC-003 en fait un signal d'observabilité et
  de rétention future, pas une condition pour lire les pages suivantes. Un ACK
  qui échoue ne doit pas bloquer la synchronisation.

## Séquence cible

```text
LOGIN ──▶ account_id + device_id
   │
BOOTSTRAP ──▶ GET /sync/snapshot ──▶ remplacement atomique ──▶ cursor ──▶ ACK
   │
SYNC ──▶ GET /sync/changes?after=cursor ──▶ appliquer ──▶ cursor ──▶ ACK
   │
WS {"cursor": N} ──▶ si N > cursor local ──▶ GET /sync/changes?after=cursor
```

Sens montant : écriture locale optimiste, mise en file, puis appel de l'endpoint
métier avec `X-WaveFlow-Operation-Id` (et `X-WaveFlow-Device-Id`). Le serveur
applique ou reconnaît le rejeu ; la modification revient ensuite par `/changes`.

## Avertissement

**Homonymie de RFC.** Le Desktop a son propre
`docs/rfcs/RFC-003-sync-architecture.md` (horloges logiques hybrides, statut
*Draft*, 2026-06-12), sans rapport avec le `RFC-003` du serveur (sync v2,
*accepted*, 2026-08-09). Les commentaires de `sync/mod.rs` renvoient au RFC-003
**desktop**. Toute consigne mentionnant « RFC-003 » doit préciser le dépôt, sous
peine de contresens.

## Rappel de périmètre

RFC-003 pose que le catalogue serveur est une **source distante séparée** : le
protocole ne synchronise que l'état possédé par le compte, n'importe jamais une
piste serveur dans le catalogue local et ne devine jamais une correspondance
locale/serveur. La réconciliation est **M5** et exige son propre RFC — liaison
automatique sur hash complet unique seulement, MBID en suggestion à confirmer,
aucun rapprochement flou par titre/artiste/durée.
