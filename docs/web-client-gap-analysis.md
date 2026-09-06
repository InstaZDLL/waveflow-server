# Le client web embarqué ↔ Navidrome : inventaire de l'écart et plan

> Relevé du 2026-09-06, établi sur `main` à `84d0f8b`, en lecture seule.
> Les chiffres viennent du code : les chemins déclarés par `#[utoipa::path]`
> sous `src/api/` et `src/media.rs` d'un côté, les appels de
> `webapp/src/api.ts` de l'autre. Navidrome est décrit d'après sa propre
> documentation ([navidrome.org/docs/overview](https://www.navidrome.org/docs/overview/)),
> pas de mémoire.
>
> Aucun code n'a été modifié pendant ce relevé.

## Constat

**Le serveur déclare 63 chemins sous `/api/v2`. Le client web en appelle 26.**

C'est le fait qui gouverne tout ce document. Ce qui manque à l'interface n'est,
pour l'essentiel, pas à construire : c'est à brancher. Sur les quinze familles
de fonctions absentes de l'écran, **onze sont déjà servies** et attendent un
appel `fetch`. Quatre seulement demandent du travail côté serveur, et trois de
ces quatre sont des fonctions que Navidrome a et que WaveFlow n'a jamais eu
l'intention d'avoir.

La deuxième observation est de forme et se déduit de la première. L'interface
paraît générique moins par ses couleurs que par sa **vacuité** : une grille, une
barre latérale, un lecteur, et rien d'autre à l'écran. Aucun en-tête de page,
aucun tri, aucun filtre, aucun survol. Navidrome ne paraît pas plus riche parce
qu'il est mieux dessiné — il paraît plus riche parce qu'il met du tri, du
filtre, du survol-lecture et de la gestion de file dans les mêmes centimètres
carrés.

Une troisième chose mérite d'être dite d'emblée, parce qu'elle change le coût de
tout le reste : **il n'y a aucune bibliothèque d'interface.** Ni Tailwind, ni
Radix, ni shadcn, ni Lucide. Trois dépendances runtime — React, React DOM,
TanStack Router — et 1 327 lignes de CSS écrites à la main, 666 lignes de
tokens de thème, des icônes SVG maison. Le rendu « template » vient du
vocabulaire employé, pas d'un cadre imposé. Il n'y a donc rien à arracher, et
chaque règle est modifiable sans combattre personne.

## 1. L'écart fonctionnel avec Navidrome

### Ce que les deux savent faire

Multi-comptes avec état par utilisateur, multi-bibliothèques avec droits par
compte, transcodage à la volée, partages par lien public, surveillance
automatique de la bibliothèque (`WAVEFLOW_SCAN_INTERVAL_SECS`), interface web
thématisable et responsive, compatibilité Subsonic/OpenSubsonic validée sur
cinq clients réels.

Sur ce socle, l'écart n'est pas fonctionnel. Il est d'interface.

### Ce que Navidrome fait et que WaveFlow ne fait pas du tout

Ces cinq points demandent du serveur, pas de l'écran. Aucun n'est commencé.

| | État dans WaveFlow |
| --- | --- |
| **Playlists intelligentes** (dynamiques, à la iTunes) | rien |
| **Import automatique des `.m3u`** | rien |
| **Scrobbling externe** — Last.fm, ListenBrainz, Maloja | rien ; `/api/v2/scrobbles` n'écrit que le journal interne |
| **Radio internet** | `getInternetRadioStations` répond un conteneur vide (`src/subsonic/mod.rs:404`) |
| **Mode jukebox** | `jukeboxRole` est déclaré `false` (`src/subsonic/nodes.rs:319`) |

Deux écarts de degré s'y ajoutent. Le **transcodage** de Navidrome se règle par
utilisateur et par lecteur ; WaveFlow n'a que des plafonds de concurrence,
`WAVEFLOW_TRANSCODE_GLOBAL_LIMIT` et `..._PER_USER_LIMIT`, sans profil de format
ou de débit par compte. Et la **traduction** : 34 langues contre deux.

### Ce que Navidrome fait, que le serveur sait déjà faire, et que l'écran ignore

C'est le gros de l'écart, et il ne coûte que du client.

| Fonction | Route déjà servie |
| --- | --- |
| Notes 5 étoiles | `GET /ratings`, `PUT /ratings/{type}/{id}` |
| Reprise de lecture | `GET /bookmarks`, `PUT`/`DELETE /bookmarks/{track}` |
| Paroles | `GET /tracks/{id}/lyrics` |
| Navigation par genre | `GET /genres`, `GET /songs?genre=` |
| Historique d'écoute | `GET /history` |
| Lecture aléatoire | `GET /songs/random` (filtrable par genre et par années) |
| En écoute maintenant | `GET /now-playing` |
| Progression de scan en direct | `GET /scans/{id}`, `GET /scans/{id}/events` (SSE) |
| Pistes d'une bibliothèque | `GET /libraries/{id}/tracks` |
| Gestion des membres | `PUT`/`DELETE /libraries/{id}/members/{user}` |
| Jetons d'API | `GET`/`POST /admin/users/{u}/tokens`, `DELETE .../{id}` |
| État du transcodage | `GET /transcode/status` |

### Ce que WaveFlow fait et que Navidrome ne fait pas

L'écart n'est pas à sens unique, et ces quatre-là n'ont pas d'écran non plus.

- **Recevoir un fichier** — [RFC-008](rfcs/RFC-008-receiving-a-file.md), négociation
  fragmentée, quota, validation. `POST /libraries/{id}/uploads` et les routes de
  session.
- **Le canvas par piste** — [RFC-009](rfcs/RFC-009-track-canvas.md), six routes.
- **Le flux d'événements de bibliothèque** — [RFC-007](rfcs/RFC-007-library-event-stream.md),
  avec appareil d'origine, rétention et acquittement.
- **La correction de tags qui survit à un rescan** — `PATCH /tracks/{id}`,
  adossée à `track_override`.

Un cinquième, invisible mais structurant : les identifiants d'album et
d'artiste sont **dérivés** des tags (`src/pid.rs`, UUID v8), donc stables d'une
installation à l'autre.

### Ce qui n'est pas un objectif

À écrire une fois pour ne pas y revenir. La **façade Subsonic est gelée** pour
`v2.0-beta` ; rien de ce plan ne doit en changer le comportement observable.
Le **client desktop** reste le consommateur privilégié des surfaces récentes :
le client web n'a pas à rattraper le desktop route pour route.

## 2. Le câblage : ce qui manque vraiment

### Onze branchements, zéro ligne de serveur

Chacun est un écran ou un contrôle, contre une route qui répond déjà. Rien à
concevoir, rien à migrer, rien à faire relire côté domaine.

1. **Étoiles** sur la piste, l'album et l'artiste. La route accepte les trois
   types ; l'écran n'en montre aucun.
2. **Paroles** dans une vue « en écoute ». La route rend `LyricsList`, donc les
   formes synchronisée et plate.
3. **Reprise** : reposer la tête de lecture où elle était. Le signet existe par
   piste.
4. **Genres** comme entrée de navigation, avec `GET /songs?genre=` pour la
   suite.
5. **Historique** et **écoutes récentes**, tous deux dans `GET /history`.
6. **Aléatoire** comme action de premier plan — c'est ce que fait un serveur de
   musique quand on ne sait pas quoi écouter, et la route filtre déjà par genre
   et par années.
7. **En écoute maintenant**, dans l'administration : qui écoute quoi, sur quel
   appareil.
8. **Progression de scan en direct.** L'écran d'administration lance un scan et
   ne dit plus rien ensuite. Le flux SSE existe.
9. **Sélecteur de bibliothèque.** Le client liste les bibliothèques dans
   l'administration mais ne cadre jamais la navigation dessus, alors que
   `GET /libraries/{id}/tracks` et les paramètres `library_id` de `/genres`,
   `/songs` et `/songs/random` sont faits pour ça. C'est le seul écart où
   Navidrome est franchement devant sur une fonction que WaveFlow possède.
10. **Membres d'une bibliothèque** : ajouter, changer de rôle, retirer.
11. **Jetons d'API**, pour qu'un compte puisse en créer sans passer par la CLI.

### Trois branchements pour les surfaces récentes

Plus lourds, parce qu'il faut dessiner l'interaction autant que l'appeler.

12. **Correction de tags** — `PATCH /tracks/{id}`. Un formulaire par piste, et
    la question de ce qu'on montre quand une correction diverge du fichier.
13. **Téléversement** — négociation, fragments, validation. C'est le plus gros :
    il faut une file, une reprise, une progression, et le drapeau
    `accepts_uploads` à respecter dans l'écran (une bibliothèque fermée ne doit
    pas offrir le bouton).
14. **Canvas** — poser, remplacer, retirer, et le lire dans le lecteur via
    `canvas-ticket` puis `canvas-stream/{ticket}`. Le discriminant de ticket
    existe déjà, donc `<video src>` fonctionnera sans en-tête.

### Ce qui demande vraiment du serveur

15. **Scrobbling externe.** C'est le seul des cinq manques Navidrome dont
    l'absence se remarque à l'usage quotidien. Il demande un client HTTP
    sortant, des identifiants par compte et une file de reprise — donc une
    décision de conception, donc une RFC avant du code.

Les quatre autres — playlists intelligentes, import `.m3u`, radio, jukebox — se
décident avant de se chiffrer. Aucun n'est un prérequis de `v2.0-beta`.

## 3. Le plan de redesign

### Le diagnostic, et pourquoi il n'est pas une affaire de goût

Le grief « ça fait shadcn » est juste sur le rendu et faux sur la cause : il n'y
a pas de shadcn. Le vocabulaire employé — fond neutre très sombre, un accent
saturé unique, cartes arrondies, bouton pleine largeur — est celui que tout le
monde produit, et il se change en CSS.

Mais la raison principale pour laquelle l'interface paraît générique est
**qu'elle est vide**. Une page qui ne porte qu'une grille ressemble à toutes les
pages qui ne portent qu'une grille. Le redesign est donc d'abord un travail de
**densité**, ce qui a l'avantage d'être vérifiable au lieu d'être discutable :
soit une page a un en-tête et des contrôles, soit elle n'en a pas.

### Les gestes, du plus rentable au moins

**Un en-tête par page.** Titre, compte d'éléments, et les contrôles qui vont
avec — tri, filtre, densité d'affichage. Aujourd'hui la grille démarre à ras
bord et il n'existe qu'un seul ordre, sans moyen d'en changer. C'est le geste
qui ferme le plus d'écart avec Navidrome, et il ne dépend d'aucune route
nouvelle.

**Des affordances sur les pochettes.** Un survol qui offre « lire » et « mettre
en file ». Actuellement il faut entrer dans un album pour lancer quoi que ce
soit.

**Un lecteur qui mérite sa barre.** Il porte précédent, pause, suivant, un
scrubber et deux durées. Il lui manque le volume, l'aléatoire, la répétition,
l'accès à la file, et le retour vers l'album en cours depuis la pochette.

**Les titres sur deux lignes.** `line-clamp: 2`. Aujourd'hui la troncature
frappe en permanence — « 50thSg発売記念〜モー… », « Armageddon - The 1st Al… »,
« Ballad of the Mer (Orches… ». Une ligne de CSS pour la moitié du problème.

**Les deux `<select>` natifs** du thème et de la langue, dans une application
entièrement stylée à la main. C'est le bord non fini le plus visible de
l'interface.

**La typographie, décidée une fois.** Le serif d'affichage est le seul geste qui
ne soit pas générique, et il est orphelin : il sert sur l'écran de connexion,
que personne ne regarde, et nulle part ensuite. Deux issues cohérentes — en
faire un système, en le portant sur les titres de page ; ou l'abandonner. La
seule mauvaise décision est de le laisser où il est.

**La carte dans la carte**, sur la connexion : deux rectangles arrondis
emboîtés dont les fonds diffèrent de trois pour cent. C'est le signal
« template » le plus fort de l'écran.

**La composition en largeur.** L'écran de connexion a été dessiné autour de
1280 px puis centré ; au-delà il flotte dans un vide qui ne travaille pas.

**La langue de l'écran de connexion.** Les libellés sont en anglais alors que
l'application tourne en français, parce que le sélecteur de langue vit *après*
la session. Il faut lire `navigator.language` avant qu'un compte existe. Ce
n'est pas du goût, c'est un défaut.

**Le vide de la barre latérale**, entre la navigation ancrée en haut et les
réglages ancrés en bas.

### Ce qu'il ne faut pas faire

Ne pas introduire de bibliothèque d'interface pour régler un problème de
vocabulaire. Trois dépendances runtime et 1 327 lignes de CSS sont un actif :
la porte automatisée du client — Biome, TypeScript, Vitest, Playwright et les
règles WCAG A/AA via `@axe-core` — tient parce que la surface est petite.

## Séquence proposée

Quatre lots, ordonnés pour que chacun soit publiable seul.

**Lot A — la densité.** En-têtes de page avec tri et filtre, `line-clamp`,
survol-lecture, selects stylés, langue au premier chargement, le vide de la
barre latérale. Zéro route nouvelle. C'est le lot qui change le plus la
perception pour le moins de risque.

**Lot B — le branchement du confort.** Étoiles, paroles, reprise, genres,
historique, aléatoire, sélecteur de bibliothèque. Onze routes qui existent, un
écran chacune ou presque. C'est ce qui ferme l'écart Navidrome.

**Lot C — le lecteur et l'exploitation.** Volume, aléatoire, répétition, file
accessible, retour vers l'album. Puis côté administration : progression de scan
en direct, en écoute maintenant, membres, jetons.

**Lot D — les surfaces récentes.** Correction de tags, puis téléversement, puis
canvas. Dans cet ordre : la correction est un formulaire, le téléversement est
une machine, le canvas suppose que le lecteur du lot C existe.

Le scrobbling externe ne rentre dans aucun lot. Il demande une RFC.

## Rappel de périmètre

Rien de ce plan ne touche la façade Subsonic, gelée pour `v2.0-beta`, ni ne
précède le seul verrou restant de cette version : rejouer Symfonium, Feishin et
DSub contre le modèle courant, comme Juliet l'a été le 2026-08-31. Le client web
n'est pas sur ce chemin critique.

## Ce qui reste ouvert

- **Le sélecteur de bibliothèque** est le seul point où Navidrome est devant sur
  une fonction que WaveFlow possède. Reste à décider si la navigation web est
  cadrée par une bibliothèque à la fois, ou agrégée avec une bibliothèque comme
  filtre.
- **Ce qu'on montre d'une correction de tags** quand elle diverge du fichier :
  la valeur corrigée seule, ou les deux avec leur provenance.
- **Le sort du serif.** Système ou abandon, mais pas le statu quo.
- **Les langues.** Deux aujourd'hui, trente-quatre chez Navidrome. La question
  n'est pas d'y arriver mais de savoir si l'infrastructure de `i18n.tsx` tient
  au-delà d'une poignée.
