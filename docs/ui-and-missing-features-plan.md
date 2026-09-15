# L'interface et les fonctions absentes : relevé et plan

> Relevé du 2026-09-15, établi sur `main` à `746e0b1`, en lecture seule.
> Deux sources, distinctes et signalées comme telles : une séance devant
> l'interface réelle — instance locale servie par le binaire de release, base
> existante, navigation privée sans extension — et une lecture du code pour en
> chercher les causes. Chaque chiffre vient du dépôt ; ce qui n'a pas été
> vérifié porte la mention.
>
> Aucun code n'a été modifié pendant ce relevé.

## 1. Deux défauts réels, trouvés dans la console

Ce sont les seuls constats de cette séance qui décrivent un comportement
fautif. Le reste du document décrit du travail non fait, ce qui n'est pas la
même chose.

### Le billet de canvas est redemandé à chaque montage

`POST /api/v2/tracks/{id}/canvas-ticket` répond `404` pour une piste sans
canvas, et le client **le redemande sans fin** — une fois par montage du
panneau, visible en rafale dans la console.

Le client traite pourtant le cas correctement : `canvasUrl`
(`webapp/src/api.ts:647`) attrape le `404` et renvoie `null`. Aucune exception
n'est levée ; ce que la console montre, c'est la requête elle-même, que le
navigateur journalise quoi qu'en fasse le JavaScript.

La cause est dans `canvasQuery` (`webapp/src/queries.ts:231`) : `staleTime: 0`
et `gcTime: 0`. Ces deux zéros ont été posés **exprès** par la PR #204 — un
billet de canvas expire, donc il ne doit jamais être servi depuis un cache.
Mais ils ne distinguent pas deux réponses de nature opposée :

- **un billet**, qui périme et ne doit pas être gardé ;
- **« cette piste n'a pas de canvas »**, qui ne périme pas dans une session.

**Correctif attendu** : mémoriser la réponse négative pour la session, et
seulement elle. Ne jamais garder un billet vivant. C'est exactement la famille
du favicon en boucle — une réponse stable redemandée indéfiniment.

### Une session morte interroge le serveur pour toujours

`GET /api/v2/now-playing` répond `401`, et recommence.

`nowPlayingQuery` (`webapp/src/queries.ts:125`) porte `refetchInterval:
30_000`. Et `call` (`webapp/src/api.ts:314`) retente **une** fois après un
`refresh()` réussi. Donc un `401` qui atteint la console signifie que le
rafraîchissement lui-même a échoué : la session est définitivement morte.

**Conséquence** : un `401` toutes les trente secondes, indéfiniment, sans que
l'utilisateur soit ramené à l'écran de connexion. L'application reste affichée,
vide, en interrogeant un serveur qui lui répond non.

**Correctif attendu** : un `401` survivant au rafraîchissement doit arrêter
l'intervalle et rendre la main à l'écran de connexion, pas boucler.

### Ce qui, dans la même console, n'est pas un défaut

- **`GET /favicon.ico 404`, une seule fois.** C'est le comportement voulu, et
  ça confirme la PR #205 en conditions réelles : plus de boucle, plus de
  coquille servie comme image.
- **`Cannot read properties of undefined (reading 'startTime')` dans
  `reportAllChanges`.** C'est l'API de **web-vitals**, qui n'est ni dans
  `package.json` ni dans `bun.lock` — les cinq dépendances du client sont
  `react`, `react-dom`, `@tanstack/react-query`, `@tanstack/react-router` et
  `@noble/hashes`. Le code fautif est injecté (`VM227`, source anonyme), donc
  il vient de l'outillage du navigateur, pas du client.
- **`ERR_CONNECTION_REFUSED`.** Le binaire était en cours de reconstruction.

## 2. L'interface : ce que la séance demande

### La marque du serveur n'est pas celle de WaveFlow

Les deux identités ont divergé, et pas sur un détail. Le logo de référence est
celui du dépôt voisin, `assets/logo.svg` de
[InstaZDLL/WaveFlow](https://github.com/InstaZDLL/WaveFlow) :

|  | Desktop (référence) | Serveur (actuel) |
|---|---|---|
| Barres | **cinq** | quatre |
| Profil | **symétrique** (140/80/40/80/140) | asymétrique, montant puis redescendant |
| Couleur | **dégradé diagonal** `#34D399` → `#10B981` → `#059669` | `#34d399` plat |
| Contenant | **aucun**, les barres sont le logo | barres évidées d'un carré arrondi plein |

**Et la marque du serveur est recopiée à trois endroits, dont aucun n'est un
SVG** :

- `webapp/src/styles.css:308` — `.brand-mark`, dessinée en CSS ;
- `webapp/src/main.tsx:120` — les quatre `<i>` que ce CSS met en forme ;
- `webapp/public/favicon.svg` — la seule version vectorielle, écrite à la main
  d'après les deux précédentes.

**Correctif attendu** : adopter l'identité du desktop, et surtout **cesser de
la redessiner**. Un seul SVG, référencé par la barre latérale comme par
l'onglet, supprime la possibilité même d'une nouvelle divergence.

**Deux réserves à traiter, pas à ignorer** :

1. **Ne pas copier le fichier tel quel pour le favicon.** Il est rendu à
   16 px : cinq barres fines sans contenant, sur un dégradé, perdent leur
   contraste sur une barre d'onglets claire. Le carré arrondi du serveur avait
   cette raison-là — il rend la marque trouvable dans une rangée d'onglets. À
   arbitrer : garder un contenant **pour le favicon seul**, en portant le
   profil à cinq barres et le dégradé.
2. **Le favicon est délibérément non thématisable** (voir le commentaire de
   `webapp/public/favicon.svg`). Une icône qui change avec le thème du lecteur
   est plus difficile à retrouver, pas plus facile. Le dégradé ne remet pas ce
   choix en cause ; le contenant, si on le garde, reste plein.

### Regrouper les réglages, et y montrer les clients enregistrés

La barre latérale empile treize entrées de navigation, puis le thème, puis la
langue, puis le compte. Thème et langue ne sont pas des destinations : ce sont
des réglages posés dans un couloir.

Attendu : une page **Réglages** qui rassemble thème, langue, le scrobbling
(aujourd'hui à `/settings/scrobbling`, déjà sous ce préfixe) — et qui montre
**les clients enregistrés**, WaveFlow comme Subsonic. La matière existe déjà et
n'est visible nulle part : les jetons d'API sont enfouis dans un dépliant de la
page Administration, et le mot de passe Subsonic dédié n'apparaît qu'à côté.
Ce sont des appareils autorisés à lire la bibliothèque ; ils méritent une vue
qui dise lesquels, depuis quand, et un bouton pour les révoquer.

### Le premier chargement

Demandé : à la place du squelette, un écran de chargement soigné portant la
marque, une à deux secondes, puis chargement direct une fois en cache.

**Réserve, et elle est importante** : ne pas imposer de durée plancher de une à
deux secondes. Un plancher rend l'application plus lente pour tout le monde,
définitivement, pour cacher un chargement qui dure 79 ms. Ce qu'il faut, c'est
un écran de marque **affiché pendant** le chargement et retiré dès qu'il est
fini, avec un minimum de l'ordre de 300 ms pour éviter un clignotement. La
différence entre les deux n'est pas cosmétique : l'une habille une attente,
l'autre en fabrique une.

### La lenteur fictive : non

Proposé en séance, à voix haute, avec l'hésitation qui va avec.

**Recommandation : ne pas le faire.** L'impression que « ça charge trop vite
pour être vrai » ne vient pas de la vitesse, elle vient de **l'absence de
transition dessinée** : le contenu se substitue d'une image à l'autre, sans
rien qui relie les deux états. La réponse est une transition — un fondu court,
un glissement — pas un `setTimeout`. Une lenteur ajoutée est une régression
qu'on ne pourra plus retirer sans que ça se remarque.

### La recherche en direct

La page Recherche exige un clic sur un bouton. Attendu : la recherche part à la
frappe.

À faire avec un anti-rebond (de l'ordre de 250 ms) et l'annulation de la
requête précédente, sinon chaque lettre est un aller-retour et les réponses
arrivent dans le désordre. Le bouton peut rester pour la validation au clavier.

### Les pages artiste

Aujourd'hui : un nom et des albums. Attendu : le niveau du client desktop —
biographie, artistes similaires.

**Ce n'est pas un travail de client, et c'est le point le plus sous-estimé du
document.** La séance a d'abord conclu que « le serveur sait déjà répondre »
parce que les méthodes sont routées. Elles le sont, et elles ne répondent rien :

| Méthode | Ce qu'elle rend réellement |
|---|---|
| `getTopSongs` | conteneur vide (`src/subsonic/mod.rs:401`) |
| `getSimilarSongs` | conteneur vide (`:402`) |
| `getSimilarSongs2` | conteneur vide (`:403`) |
| `getArtistInfo`, `getArtistInfo2` | `artist_info` cherche l'artiste **pour son seul refus** — la sémantique 404 — puis renvoie un conteneur vide (`src/subsonic/browse.rs:79`) |
| `getAlbumInfo`, `getAlbumInfo2` | l'identifiant de sortie s'il existe, rien d'autre |

Et la raison est écrite dans le code, comme une politique et non comme un
oubli : **« WaveFlow queries no remote source, so notes and biography images
stay absent. »** Il n'y a ni biographie ni artiste similaire **nulle part** —
ni en base, ni dans les services, ni dans une migration. Chez Navidrome ces
données viennent de Last.fm et de MusicBrainz.

**Conséquence sur l'estimation** : une page artiste au niveau du desktop exige
une capacité qui n'existe pas — **une source de métadonnées externe**, avec son
cache, sa limitation de débit, sa dégradation quand elle est injoignable, et
une question de vie privée à trancher (un serveur auto-hébergé qui interroge un
tiers sur la bibliothèque de son propriétaire, par défaut ou sur consentement).
Cela relève d'une **RFC**, pas d'un ticket d'interface.

**Et la réponse est probablement déjà conçue, ailleurs.** Le dépôt
[InstaZDLL/waveflow-plugins](https://github.com/InstaZDLL/waveflow-plugins)
décrit des **composants WebAssembly en bac à sable** : un seul `plugin.wasm`
portable, exécuté sous wasmtime avec un budget de carburant et de mémoire, et
une **liste blanche HTTP déclarée au manifeste que l'hôte impose** — un plugin
n'atteint que les hôtes qu'il a demandés. Le registre épingle version et
`blake3`. Deux plugins de ce genre existent déjà, `apple-artwork` et
`spotify-canvas`, et le README affirme que le même fichier tourne « on Windows,
macOS, Linux, **and the server** ».

**Le serveur, lui, n'en sait rien** : aucune occurrence de `plugin` sous
`src/`. La bonne RFC n'est donc sans doute pas « le serveur appelle Last.fm »
mais **« le serveur héberge le moteur de plugins »** — la permission, le bac à
sable et la question de vie privée y sont déjà traités par construction, et une
source de métadonnées devient un plugin parmi d'autres plutôt qu'une dépendance
de plus dans le binaire. C'est plus ambitieux et plus juste ; à trancher dans
la RFC, pas ici.

Reste faisable tout de suite et sans source externe : les **titres les plus
écoutés** d'un artiste, que l'historique local sait calculer — ce qui remplit
`getTopSongs` par la même occasion.

### Ce que le serveur sait déjà, et ne dit pas

Les métadonnées techniques sont **stockées et déjà projetées en SQL** —
`codec`, `bitrate`, `sample_rate`, `bit_depth`, `channels`, `duration_ms`,
`size` figurent dans les migrations et dans la projection `song_select!`.

Mais **elles ne sortent pas** : aucune ne paraît dans les réponses de
`src/api/`, et `webapp/src/api.ts` ne les connaît pas. Donc informations de
fichier et barre de qualité **ne demandent aucune migration** — seulement de
les sérialiser, d'étendre le type client, et de dessiner. C'est la plus grosse
économie du document.

Symétriquement, une réserve sur la page Réglages : les jetons ne sont listés
que par une route **d'administration** — `/api/v2/admin/users/{username}/tokens`.
Montrer à un utilisateur **ses propres** clients enregistrés demande donc une
route non administrateur, portée sur soi. Petite, mais c'est du serveur.

### Ce qu'on ne voit pas d'un fichier

Absents de l'interface web, présents sur le desktop :

- **les informations du fichier** — conteneur, codec, débit, fréquence,
  profondeur ;
- **une barre de qualité** qui les résume d'un coup d'œil ;
- **une vue paroles** ;
- **une vue immersive**.

Les paroles sont un cas à part : le serveur les sert (`getLyrics`,
`getLyricsBySongId`), le client les lit sur « En écoute », mais il n'existe pas
de vue dédiée.

### L'espacement, et le « carton »

Constat de séance : des menus trop serrés, et une interface qui « fait
carton » — des rectangles empilés, sans hiérarchie ni respiration. La référence
citée est TPI-Flow.

Ce point n'est pas une liste de correctifs : c'est une passe de conception. Il
mérite d'être traité comme telle — un relevé des espacements, des tailles et
des niveaux de contraste réellement utilisés, puis une échelle choisie, plutôt
qu'une retouche écran par écran.

**Un indice de là où ça se joue** : `webapp/src/styles.css` fait cohabiter
**deux nomenclatures**. D'un côté des rôles — `--panel`, `--panel-soft`,
`--panel-strong`, `--line`, `--line-strong`, `--muted` ; de l'autre des
surfaces nommées par leur apparence — `--color-surface-dark`,
`--color-surface-light`, et leurs variantes `-elevated`. Deux systèmes pour la
même chose, c'est précisément ce qui produit un écran « en carton » : plus
aucune règle ne dit lequel employer, donc chaque écran tranche pour lui-même.
Il n'existe **aucune échelle d'espacement** — ni variable, ni pas — alors
qu'une échelle est exactement ce dont ce constat a besoin.

L'issue #63 (close) parlait déjà d'unifier ces surfaces sur un paquet
`@waveflow/design-tokens` en OKLCH. Ce paquet **n'est pas une dépendance du
client** et le CSS ne contient aucun `oklch`. À vérifier avant de choisir :
reprendre ce paquet, ou poser l'échelle ici.

## 3. Ce qui manque, côté fonctions

### Dans le client web

- **Aucune liste « Titres ».** On navigue par album, artiste, genre, favoris,
  historique, aléatoire — jamais « toutes mes pistes, triées ». Le composant
  existe (`SongTable`, utilisé à cinq endroits) ; la route non. C'est le manque
  qu'un utilisateur venant de Navidrome remarque en premier.

  **Et le serveur ne sait pas répondre non plus.** L'issue #179 disait que
  `GET /api/v2/songs` exigeait un `genre` et ne pouvait donc rien lister ; elle
  a été close le 2026-09-10 **en renommant la route** en
  `/api/v2/songs/by-genre`. Le contrat est devenu honnête, la fonction n'est
  pas apparue : il ne reste que `by-genre` et `random`. La liste « Titres »
  demande donc **une nouvelle route serveur** — paginée et triable — avant
  toute page. C'est une correction à l'estimation : ce point traverse le fil.
- **Aucun import `.m3u`.** Rien dans le dépôt. C'est ce qui permet d'arriver
  avec une collection existante : une barrière à l'adoption, pas un confort.
- Pas de playlists intelligentes, pas de profils de transcodage.

### Dans la façade Subsonic

Cinquante-neuf méthodes sont servies. Les absences, avec ce que le serveur en
dit :

| Absent | Rôle déclaré | État |
|---|---|---|
| Radios internet (create/update/delete) | — | `getInternetRadioStations` renvoie un conteneur vide (`src/subsonic/mod.rs:404`) |
| Jukebox (`jukeboxControl`) | `jukeboxRole: false` | rien |
| Podcasts (7 méthodes) | `podcastRole: false` | rien |
| Chat (`getChatMessages`, `addChatMessage`) | `commentRole: false` | rien |
| Vidéo (`getVideos`, `hls`, `getCaptions`…) | `videoConversionRole: false` | rien |
| Upload par Subsonic | `uploadRole: false` | existe sur `/api/v2`, pas exposé |
| `search` (v1 hérité) | — | `search2` et `search3` servis |

**Aucune n'est un mensonge** : chaque rôle est déclaré `false`, donc un client
n'offre pas le bouton. La radio est la seule qui dépasse — un conteneur vide
dit « zéro station » et non « pas supporté », ce qui est le seul choix que le
protocole laisse.

### Dans le serveur

- **Rien n'est compressé.** `tower-http` est compilé avec `cors`,
  `request-id`, `timeout`, `trace` ; aucune couche de compression n'existe. Le
  client part brut : **497 ko au lieu d'environ 152 ko**. Une feature et une
  couche. Gain nul sur un réseau local, décisif depuis l'extérieur.
- **L'artwork animé n'existe pas.** Le canvas vidéo, lui, est livré
  (RFC-009) — les deux sont régulièrement confondus.
- **Le scrobbling externe est complet côté code** : `lastfm`, `listenbrainz`,
  `maloja`. Ce qui bloque Last.fm est l'enregistrement d'une application chez
  eux, pas une ligne à écrire.

### Un écart de structure, sans effet visible

Quatre écrans — éditeur de tags, panneau canvas, upload, scrobbling —
importent `pages.tsx` pour **trois primitives** : `Loading`, `PageHeader`,
`useReload`. Ils tirent ainsi 70 ko de module et, par transitivité,
`player.tsx`.

**Mesuré, et contre l'intuition** : les extraire puis découper par route
économiserait **environ 18 ko sur 466** (4 %). React et TanStack pèsent
**276 ko, soit 59 % du paquet**, et sont exigés au premier octet quelle que
soit la page. L'extraction vaut pour la propreté du graphe de modules, **pas
pour la vitesse** — et un découpage par route ne doit pas être vendu comme une
optimisation.

## 4. Un ordre, si l'on veut en choisir un

1. **Les deux défauts de la section 1.** Ce sont des comportements fautifs, pas
   des fonctions manquantes, et le second laisse une session morte à l'écran.
2. **La marque**, **la recherche en direct** et **la liste « Titres »**. Peu de
   code, très visibles ; la liste a déjà son composant, et la marque est le
   seul point du document qui soit une divergence d'identité plutôt qu'un
   manque.
3. **La compression du serveur.** Une feature, une couche, un test.
4. **La page Réglages**, clients enregistrés compris — elle range ce qui
   traîne et expose ce qui est aujourd'hui caché.
5. **L'écran de chargement de marque** et les transitions. Sans plancher
   artificiel.
6. **Les titres les plus écoutés d'un artiste**, calculés sur l'historique
   local — la seule moitié de la page artiste qui ne demande pas de source
   externe, et elle remplit `getTopSongs` au passage. **La biographie et les
   artistes similaires sortent de cette liste** : ils exigent une RFC sur une
   source de métadonnées externe.
7. **La passe de conception** — espacements, hiérarchie. À traiter comme un
   sujet, pas comme une liste.
8. **Informations de fichier, barre de qualité, vue paroles, vue immersive.**
9. **Import `.m3u`**, puis les radios internet.
10. Playlists intelligentes, jukebox, podcasts : de vrais projets.

## 5. Le découpage en pull requests

Onze pull requests, plus une RFC. Les tailles emploient l'échelle des étiquettes
du dépôt (`size: xs` < 10 lignes, `s` 10-50, `m` 50-200, `l` 200-500,
`xl` > 500) et restent des **estimations**, tests compris.

| # | Issue | Intitulé | Ce qu'elle porte | Taille |
|---|---|---|---|---|
| 1 | **#206**, **#207** | Une réponse stable ne se redemande pas | Les deux défauts de la section 1 | `m` |
| 2 | **#208** | Une seule marque, et un écran qui la porte | Identité du desktop, trois copies réduites à un SVG, écran de chargement et transitions | `l` |
| 3 | **#210** | Le catalogue se laisse parcourir | Route serveur listant les pistes, page « Titres », recherche à la frappe, extraction des trois primitives hors de `pages.tsx` | `l` |
| 4 | **#209** | Le serveur compresse ce qu'il envoie | Une feature `tower-http`, une couche, un test | `s` |
| 5 | **#211** | Ce qu'un fichier est, et une barre qui le dit | Sérialiser cinq champs déjà projetés, étendre le type client, dessiner | `l` |
| 6 | **#212** | Les réglages cessent de traîner dans un couloir | Page Réglages, clients enregistrés, route de jetons portée sur soi | `l` |
| 7 | **#213** | Les paroles et la vue immersive | Deux vues sur « En écoute » | `l` |
| 8 | **#214** | La passe de conception | Échelle d'espacement, nomenclature unique, hiérarchie | `xl` |
| 9 | **#215** | Les plus écoutés d'un artiste | Calcul sur l'historique local ; remplit `getTopSongs` | `l` |
| 10 | **#216** | Import `.m3u` | Analyse, résolution des chemins, serveur et client | `xl` |
| 11 | **#217** | Les radios internet | Migration, trois écritures Subsonic, `/api/v2`, page | `xl` |

**Hors PR — une RFC, suivie par #218** : le moteur de plugins côté serveur,
dont dépendent la biographie et les artistes similaires. Voir la section « Les
pages artiste ».

La première ligne porte **deux** issues parce qu'elle porte deux défauts
distincts : une issue décrit un problème, une PR peut en fermer plusieurs.

### L'ordre, et ce qui contraint

- **1 à 4 ne dépendent de rien** et peuvent partir ensemble. Deux vrais
  défauts, l'identité, les deux manques les plus visibles et le seul gain de
  transfert du document.
- **7 suppose 2** : la vue immersive réemploie l'écran de marque.
- **8 vient après 3, 5, 6 et 7**, sinon la passe se refait sur du balisage qui
  a changé entre-temps.
- **9 est la seule moitié de la page artiste** qui n'attend pas la RFC.

### Deux regroupements refusés, et pourquoi

La consigne était de grouper au maximum. Deux exceptions, assumées :

1. **La compression reste seule** (4) plutôt que de rejoindre `.m3u` ou les
   radios, l'autre travail Rust. La grouper ferait attendre quarante lignes à
   fort rendement derrière un chantier de plusieurs centaines.
2. **5 et 7 restent séparées** bien qu'elles touchent le même écran. La 5
   **change la forme du fil**, la 7 non. Mêler un changement de wire à du
   travail d'interface est exactement ce qui a laissé passer les quatre
   défauts de la PR #203 : les mocks confirmaient les types au lieu de les
   contredire. Une PR qui touche le fil mérite son propre regard.

## Ce qui n'a pas été vérifié

Un seul point, relevé à l'écran et **non instruit** : la recherche « Tz » rend
deux albums, `abouTZU` (crédité `TZUYU, PENIEL`) et `abouTZU - EP` (crédité
`TZUYU`), et l'EP affiche deux pistes numérotées 3 et 6. Ce peut être
parfaitement exact — deux sorties distinctes, une extraction partielle — ou le
signe d'une identité d'album qui se scinde sur le crédit. Les identifiants
d'album étant dérivés des tags (`WAVEFLOW_PID_ALBUM`), la question se tranche
en lisant les tags des fichiers, pas en lisant l'écran. À faire avant d'en
conclure quoi que ce soit.
