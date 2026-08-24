# Audit OpenSubsonic / API v2 — cinquième passe

Périmètre : `main` à `1eecc10`, soit les PR #121 et #122 (21 commits) par-dessus
les 32 des passes précédentes. Ce document remplace le rapport de quatrième
passe.

Vérification statique. `cargo clippy` et `cargo test` ne peuvent pas s'exécuter
ici — `index.crates.io` est bloqué par la politique de sortie réseau et aucun
registre n'est en cache. `CI (Rust)` a réussi sur `4110f22`, le merge de #121 ;
elle était encore en cours sur `1eecc10` au moment de la rédaction.

---

## 1. Les deux priorités sont fermées

### 1.1 L'élévation OAuth

`Access::Unrestricted` nomme la catégorie qui manquait : émettre une référence.
`granted_by` retourne `false` pour toute liste non vide, et seul le retour
anticipé sur liste vide l'accorde — donc une session, une concession OAuth ou un
jeton sans portées peuvent émettre, un jeton restreint non.

C'est la bonne forme, et pour la bonne raison : ce n'est pas un rôle mais une
restriction. Un compte ordinaire appaire toujours ses propres appareils depuis
sa propre session, ce à quoi le flux sert ; ce qui est refusé, c'est de le faire
au nom d'une référence délibérément rétrécie.

Une seule route la porte (`src/http.rs:1116`) — j'ai vérifié qu'aucune autre
route n'émet de référence sous une autorité moindre : `create_api_token` et
`set_subsonic_credential` sont `Admin`, `auth/login` exige le mot de passe,
`auth/refresh` exige le jeton de rafraîchissement.

Le correctif durable — porter les portées à travers la concession, sur
`oauth_authorization` puis `session` — reste dû, et l'enum comme la RFC le
disent. C'est la bonne façon de laisser une dette : nommée là où on la
retrouvera.

### 1.2 La matrice de compatibilité a été rejouée

Quatre clients rejoués le 2026-08-19 contre le contrat courant — Symfonium,
Feishin, DSub, Juliet — chacun sur un appareil ou un poste réel, et chacun **lu
depuis l'état du serveur plutôt que depuis l'affichage du client**. C'est la
distinction qui rend la campagne crédible.

Substreamer est explicitement **hors du jeu rejoué** : le build ne se lance plus
sur l'environnement courant, sa ligne reste comme preuve historique contre
l'ancien contrat et ne compte pas pour le prochain tag. Juliet prend sa place et
déplace au passage la vérification iOS d'un émulateur vers un iPhone physique.

Cette honnêteté-là se retrouve partout dans le document : les méthodes non
appelées par un client sont enregistrées comme non exercées plutôt que déduites
comme passantes, le FLAC que DSub ne termine jamais est consigné comme non résolu
avec la preuve que le serveur n'est pas en cause, et deux affirmations du brief
interne ont été corrigées **par** les runs plutôt que confirmées par eux.

---

## 2. Ce que la campagne a rapporté

Quatre défauts serveur, qu'aucun test automatique n'aurait produits. C'est
l'argument pour la validation client réelle, fait concrètement plutôt qu'affirmé.

| Défaut | Trouvé par | Ce qu'il fallait pour le voir |
|---|---|---|
| Un transcodage froid refusait `Range: bytes=0-` | Feishin | un navigateur, qui ouvre toute ressource ainsi |
| La sonde `bytes=0-1` d'AVFoundation restait classée comme un seek après le premier correctif | Juliet | un lecteur iOS, qui sonde avant de lire |
| `createPlaylist` avec `playlistId` ajoutait au lieu de remplacer | Feishin | un client qui édite en renvoyant ce qui reste |
| Un album crédité à deux artistes pendait sous une troisième entité nommée d'après la chaîne jointe | DSub | un client qui navigue par index d'artiste |

Les deux premiers se manifestaient de la manière la plus trompeuse qui soit :
la première lecture d'une piste échouait, la seconde réussissait sur le cache
que la requête échouée venait de construire. Un test qui joue deux fois passe.

### Qualité des correctifs — revue

**Plages d'octets.** `starts_at_the_first_byte` et `parse_range` rognent les
deux bornes de la même façon, et rejettent tous deux le multipart : une
graphie acceptée à froid ne peut pas être refusée une fois le cache présent.
`bytes=0-1,4-5` est bien refusé — le second segment ne s'analyse pas comme un
entier. Répondre 200 avec le flux entier à une plage d'ouverture est permis par
HTTP, et c'est le seul choix disponible : avant l'existence du transcodage il
n'y a pas de longueur totale à mettre dans un `Content-Range`.

**Identité d'album.** Quatre commits, ce qui traduit une itération honnête
plutôt qu'un tâtonnement : en corrigeant le rattachement à l'artiste, ils ont
trouvé que canonicaliser le crédit joint effaçait les frontières entre crédits,
si bien que `A; B C` et `A; B; C` répondaient la même clé et fusionnaient. La
forme finale canonicalise crédit par crédit et rejoint sur un séparateur que la
forme canonique ne peut pas produire.

La passe de réparation mérite d'être signalée : elle rattrape les bibliothèques
indexées par une version antérieure **sans reconstruction**, puisque les
fichiers n'ont pas changé et que rien ne serait jamais réindexé. Et elle traite
la collision de clé en déplaçant les pistes puis en supprimant la ligne
obsolète, plutôt que par un `UPDATE` qui violerait l'unicité et ferait échouer
le scan entier. Son filtre `LIKE '%;%'` correspond exactement au séparateur de
`split_values` — j'ai vérifié, il n'est pas plus étroit que le découpeur.

**Le correctif que je retiens** est celui qu'aucun client n'a signalé.
Ajouter `clear_tracks` à la charge d'intention sans condition aurait changé
l'empreinte de **toutes** les mises à jour de playlist déjà enregistrées par
cette version, transformant le rejeu d'un client à travers la mise à niveau en
conflit. Le champ n'est ajouté que lorsqu'il est posé. C'est le genre de défaut
qui ne se manifeste qu'en production, chez un utilisateur, une seule fois.

---

## 3. Ce qui reste

> **Addendum du 2026-08-20.** Ce rapport décrit `main` à `1eecc10`. La PR #123
> a depuis fermé les **quatre** lignes de cette section : les portées traversent
> la concession OAuth (`scopes_json` sur `oauth_authorization` et sur `session`,
> deux migrations), `sortName` existe sur album et artiste et se redérive en fin
> de scan, `getMusicDirectory` répond pour les pistes sans album, et le 400 de
> `/api/v2/search` est annoté. `Access::Unrestricted` a disparu avec elles : le
> portage des portées remplace la restriction en bloc décrite au §1.1, qui n'y
> tenait que le temps de la migration. `getLicense` garde sa date en dur,
> délibérément.
>
> La question de cadrage en fin de section — **quatre clients rejoués contre
> cinq** — reste ouverte : c'est une décision, pas un correctif.

> **Addendum du 2026-08-23.** Le point 2 ci-dessous est caduc pour sa moitié
> la plus lourde. La PR #126 a livré `roles[]`, `contributors[]` et
> `displayComposer` avec les colonnes qu'ils réclamaient — treize rôles, un
> sous-rôle d'instrument, et un album qui pend de chacun de ses artistes
> crédités. `sortName` était déjà arrivé avec la PR #123. Les champs de sortie
> d'album
> (`originalReleaseDate`, `releaseDate`, `releaseTypes[]`, `recordLabels[]`,
> `discTitles[]`) ont suivi le 24 août 2026 : les quatre premiers pendent de
> l'album, `discTitles[]` se dérive des pistes disponibles comme les genres, et
> les trois tableaux sont émis vides plutôt qu'absents. Les deux dates sont
> omises quand aucun tag ne les nomme, comme le fait la référence — un
> `ItemDate` sans année n'est pas une date, et les tableaux portent déjà le
> signal de présence du groupe. **Ce point est clos.**
>
> La question de cadrage du §5.1 est close : les quatre clients ont été
> rejoués le 23 août 2026 contre le modèle aligné, et
> [`subsonic-compatibility.md`](subsonic-compatibility.md) porte le résultat.

Rien de structurel. La liste tient en quatre lignes, et deux d'entre elles sont
des dettes nommées plutôt que des défauts.

1. **Le correctif OAuth durable** — porter les portées à travers la concession.
   Deux colonnes, deux migrations. Le chemin est fermé aujourd'hui ; ce qui
   reste, c'est que la propriété soit structurelle et non locale à une route.
2. ~~**Champs `AlbumID3` et `ArtistID3`** demandant des colonnes.~~ Livrés :
   `sortName` par la PR #123, `moods[]`, `explicitStatus`, `roles[]`,
   `contributors[]` et `displayComposer` par la PR #126, puis
   `originalReleaseDate`, `releaseDate`, `releaseTypes[]`, `recordLabels[]` et
   `discTitles[]` le 24 août 2026.
3. **`song.parent` retombe sur `library_id`** sans album (`src/subsonic.rs:1706`) :
   un `getMusicDirectory` sur cet identifiant ne renverra pas la piste.
4. **Deux inexactitudes de surface** : `getLicense` expire en dur au
   2099-12-31, et `search_catalog` n'annote pas son 400 sur `q` manquant alors
   que `/api/v2/songs` vient de le faire.

Un point de cadrage plutôt qu'un défaut : le prochain tag reposera sur **quatre**
clients rejoués, pas cinq. Le document le dit lui-même et n'essaie pas de faire
compter Substreamer. Si cinq est le seuil voulu, il manque un cinquième client
courant ; si quatre suffit, cela vaut d'être écrit comme une décision.

---

## 4. Verdict

Il n'y a plus d'écart de capacité entre les deux surfaces, plus de défaut
d'autorisation connu, et plus de dette de validation.

| Axe | Gagnant | Écart |
|---|---|---|
| Authentification, sessions, révocation, portées | v2 | large |
| Synchro incrémentale, idempotence | v2 | large |
| Contrat typé, exploitation | v2 | large |
| Découverte, genres, favoris, marque-pages, aléatoire | égalité | fermé |
| Écosystème client | Subsonic | structurel |

Les quatre premières passes de cet audit décrivaient des écarts. Celle-ci
décrit une finition. Ce qui reste au §3 n'empêche pas un tag : ce sont des
champs optionnels correctement déclarés absents, une dette d'architecture
nommée, et deux détails cosmétiques.

---

## 5. Priorités proposées

> **Périmé au 2026-08-23, conservé tel quel.** Les points 1 et 2 sont faits :
> les quatre clients ont été rejoués, et `sortName` est livré. Voir
> l'addendum du §3. Cette liste décrit ce qui restait au moment de l'audit,
> pas ce qui reste aujourd'hui.

1. Trancher explicitement la question des quatre clients contre cinq, puisque
   c'est la seule chose entre l'état actuel et un tag.
2. `sortName` sur `album` et `artist`.
3. Le correctif OAuth durable, avant que d'autres routes n'aient à raisonner
   sur les portées.
4. Les trois points cosmétiques du §3.

---

Sources OpenSubsonic consultées :
[tokenInfo](https://opensubsonic.netlify.app/docs/endpoints/tokeninfo/),
[apiKeyAuthentication](https://opensubsonic.netlify.app/docs/extensions/apikeyauth/),
[Child](https://opensubsonic.netlify.app/docs/responses/child/),
[AlbumID3WithSongs](https://opensubsonic.netlify.app/docs/responses/albumid3withsongs/).
