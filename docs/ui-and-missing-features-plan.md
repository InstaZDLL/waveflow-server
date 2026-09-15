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

**Le serveur sait déjà répondre** : `getArtistInfo`, `getArtistInfo2`,
`getSimilarSongs`, `getSimilarSongs2` et `getTopSongs` sont tous servis par la
façade. Ce qui manque est côté client, et côté `/api/v2` il faut vérifier que
l'équivalent existe avant de promettre la page.

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

## 3. Ce qui manque, côté fonctions

### Dans le client web

- **Aucune liste « Titres ».** On navigue par album, artiste, genre, favoris,
  historique, aléatoire — jamais « toutes mes pistes, triées ». Le composant
  existe (`SongTable`, utilisé à cinq endroits) ; la route non. C'est le manque
  qu'un utilisateur venant de Navidrome remarque en premier.
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
6. **Les pages artiste**, une fois vérifié ce que `/api/v2` sait répondre.
7. **La passe de conception** — espacements, hiérarchie. À traiter comme un
   sujet, pas comme une liste.
8. **Informations de fichier, barre de qualité, vue paroles, vue immersive.**
9. **Import `.m3u`**, puis les radios internet.
10. Playlists intelligentes, jukebox, podcasts : de vrais projets.

## Ce qui n'a pas été vérifié

Un seul point, relevé à l'écran et **non instruit** : la recherche « Tz » rend
deux albums, `abouTZU` (crédité `TZUYU, PENIEL`) et `abouTZU - EP` (crédité
`TZUYU`), et l'EP affiche deux pistes numérotées 3 et 6. Ce peut être
parfaitement exact — deux sorties distinctes, une extraction partielle — ou le
signe d'une identité d'album qui se scinde sur le crédit. Les identifiants
d'album étant dérivés des tags (`WAVEFLOW_PID_ALBUM`), la question se tranche
en lisant les tags des fichiers, pas en lisant l'écran. À faire avant d'en
conclure quoi que ce soit.
