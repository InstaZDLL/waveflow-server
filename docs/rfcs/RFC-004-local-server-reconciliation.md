# RFC-004 — Réconciliation locale ↔ serveur

- **Statut** : Proposed
- **Date** : 2026-08-17
- **Auteurs** : projet WaveFlow
- **Dépend de** : [RFC-002](RFC-002-waveflow-server-v2.md),
  [RFC-003](RFC-003-waveflow-sync-v2.md) et du RFC-005 Desktop
  `remote-source-and-sync-v2`

## Problème

WaveFlow Desktop présente aujourd'hui deux catalogues séparés : ses fichiers
locaux et le catalogue autoritaire de WaveFlow Server. Cette séparation est
correcte pour M4, mais elle empêche de reconnaître qu'un fichier local est la
même copie binaire qu'une piste distante. Elle empêche notamment de convertir
explicitement une playlist d'une source vers l'autre ou de préférer le fichier
local lorsqu'il est disponible.

La réconciliation ne doit jamais fabriquer une identité à partir de
métadonnées. Un titre, un artiste, une durée ou même un ensemble de ces valeurs
peut désigner plusieurs enregistrements, éditions ou encodages.

## Décision 1 — le lien est une donnée Desktop

Le premier incrément M5 ne modifie ni le catalogue serveur ni `/api/v2`.
WaveFlow Server publie déjà pour chaque piste son UUID stable, sa taille et son
`full_hash`, défini comme BLAKE3 non keyed, hexadécimal, sur le fichier entier.
Le Desktop possède seul l'identifiant de sa piste locale et l'accès à son
fichier ; il persiste donc le lien dans la base du profil concerné.

Le lien est strictement un-à-un et contient au minimum :

- l'identifiant local stable ;
- l'identifiant distant, scoped par le profil et le serveur ;
- la méthode (`exact_full_hash` ou `confirmed_mbid`) ;
- la preuve utilisée et les dates de confirmation/vérification ;
- un état `confirmed`, `stale` ou `rejected`.

Les contraintes uniques portent sur les deux identifiants. Une copie locale ou
distante supplémentaire est une ambiguïté à résoudre, jamais un deuxième lien
automatique. Une synchronisation multi-appareils de ces liens demanderait un
contrat serveur séparé ; elle est hors du premier incrément M5.

## Décision 2 — seul un hash complet exact et unique lie automatiquement

Le `track.file_hash` actuel du Desktop n'est pas comparable à `full_hash` : il
inclut la taille et n'échantillonne que le début et la fin des gros fichiers.
Le Desktop doit utiliser son calcul BLAKE3 intégral déjà disponible.

Pour éviter de relire toute la bibliothèque :

1. indexer les pistes distantes accessibles par `(size, full_hash)` ;
2. ne calculer le hash complet que pour les fichiers locaux dont la taille a au
   moins un candidat distant ;
3. lier automatiquement uniquement lorsqu'un seul fichier local accessible et
   une seule piste distante accessible partagent le hash complet ;
4. présenter toute multiplicité comme une candidature à confirmer.

Le hash empreinte le fichier, pas l'audio décodé. Deux encodages ou deux fichiers
portant des tags différents ne correspondent donc pas, même s'ils représentent
le même enregistrement.

## Décision 3 — MBID reste une candidature confirmée

Un identifiant MusicBrainz ne provoque jamais de liaison automatique : plusieurs
pressages, encodages ou copies peuvent partager un identifiant d'enregistrement.
Une correspondance MBID peut seulement créer une candidature que l'utilisateur
confirme après avoir vu les deux sources.

Le serveur ne stocke actuellement aucun MBID. La branche MBID ne peut commencer
qu'après l'ajout explicite des identifiants canoniques nécessaires au scanner,
au catalogue et au contrat public. En leur absence, M5 fonctionne entièrement
avec le hash complet ; il ne déduit pas un MBID depuis le titre ou l'artiste.

## Décision 4 — aucun rapprochement flou

Le titre, l'artiste, l'album, la durée, le numéro de piste, la taille seule et
toute combinaison de ces champs sont interdits pour confirmer un lien. Ils
peuvent uniquement aider l'utilisateur à lire une proposition déjà fondée sur
un hash exact ambigu ou un MBID.

Un candidat rejeté ne réapparaît pas tant que sa preuve (hash ou MBID) n'a pas
changé. Cette mémoire évite de harceler l'utilisateur avec la même suggestion.

## Décision 5 — survie aux déplacements et changements

Le lien référence les identifiants stables des deux catalogues, jamais leurs
chemins ou leurs métadonnées. Il survit donc à un déplacement et à un changement
de tags lorsque les scanners conservent les identifiants.

Si les octets du fichier changent et que son hash complet ne correspond plus à
la preuve enregistrée, le lien devient `stale`. Il n'est ni supprimé ni dirigé
silencieusement vers une autre piste. Une nouvelle vérification ou confirmation
est nécessaire.

Une piste indisponible conserve son lien. La disponibilité décide seulement de
la source de lecture utilisable à cet instant.

## Décision 6 — politique de lecture proposée

Pour une piste liée, le Desktop préfère le fichier local lorsqu'il est
disponible et lisible, puis bascule sur le flux serveur. L'interface indique la
source active et permet de la forcer pour la session courante. Une piste non
liée conserve le comportement de sa source d'origine.

Le lien ne transforme pas la piste locale en piste serveur et ne fusionne pas
les catalogues. Les UUID distants continuent de nommer les mutations et données
utilisateur détenues par le serveur.

## Décision 7 — aucune fusion implicite des données utilisateur

Créer ou confirmer un lien ne modifie aucun favori, note, historique, compteur,
playlist ou file d'attente.

- **Favoris et notes** : restent propres à leur source. Une copie explicite peut
  être proposée dans un sens choisi par l'utilisateur ; il n'existe pas de
  règle automatique de victoire.
- **Historique et compteurs** : les événements historiques restent dans leur
  autorité d'origine. Un affichage peut calculer un total combiné sans le
  persister. Une nouvelle lecture peut alimenter une fois l'historique local et,
  si elle possède un lien distant, émettre un scrobble idempotent vers le
  serveur ; ce sont deux projections du même événement, pas deux lectures.
- **Playlists** : une action explicite publie une playlist locale vers le
  serveur ou matérialise une playlist serveur localement. Un aperçu signale les
  pistes non liées ou ambiguës avant validation. Il n'existe aucune fusion
  silencieuse de listes ordonnées.
- **File d'attente** : la source de lecture active reste propriétaire de sa
  file. Aucun lien ne fusionne automatiquement les deux files.

## Sécurité et isolation

Le Desktop ne considère que les pistes distantes visibles par le compte lié.
Un retrait d'accès rend le lien inutilisable sans révéler de nouvelles données.
Les chemins locaux ne quittent jamais l'appareil. Le premier incrément n'envoie
au serveur ni liens, ni inventaire local, ni nouveau hash.

## Parcours utilisateur minimal

1. lancer une analyse des correspondances depuis la source distante ;
2. voir les liens exacts automatiques, les candidats ambigus et les rejets ;
3. confirmer ou rejeter un candidat ;
4. voir et modifier la préférence de lecture d'une piste liée ;
5. convertir explicitement une playlist avec un aperçu des éléments non liés ;
6. relancer la vérification après une modification de fichier ou d'accès.

## Validation

La porte M5 exige au minimum :

- correspondance exacte unique et calcul intégral vérifié sur des fichiers
  MP3, AAC, FLAC et WAV ;
- même taille mais contenu différent : aucun lien ;
- doublons locaux ou distants : aucune liaison automatique ;
- déplacement local et distant : lien conservé ;
- changement de tags modifiant les octets : lien `stale`, jamais réassigné ;
- métadonnées identiques sans hash exact : aucun lien ;
- suppression d'appartenance à une bibliothèque : aucune fuite de catalogue ;
- conversion de playlist avec pistes liées, non liées et ambiguës ;
- confirmation d'un lien : aucune mutation implicite des données utilisateur ;
- fichiers réels de la bibliothèque de validation, sans aucune écriture de tags.

## Séquence d'implémentation

1. migration et repository Desktop pour les liens ;
2. découverte par taille puis hash complet, avec annulation et progression ;
3. écran de confirmation et gestion des états ;
4. sélection local-first avec repli serveur ;
5. conversion explicite des playlists ;
6. actions explicites sur favoris/notes et affichage des historiques ;
7. MBID seulement après un contrat de données dédié ;
8. portabilité multi-appareils seulement dans un RFC ultérieur.

## Arbitrages à accepter avant implémentation

Ce RFC propose les valeurs conservatrices suivantes :

1. **lecture local-first**, avec repli serveur ;
2. **aucune copie automatique** des favoris ou notes lors d'un lien ;
3. historiques stockés séparément, avec total combiné seulement en affichage ;
4. liens **locaux au profil Desktop** pour le premier incrément M5.

Changer l'un de ces quatre choix modifie l'expérience ou le modèle de données ;
ils doivent donc être approuvés avant de passer le RFC à `Accepted` et de
commencer l'implémentation.

## Hors périmètre

- reconnaissance acoustique ou hash de l'audio décodé ;
- rapprochement flou fondé sur les métadonnées ;
- réécriture des tags ou des fichiers audio ;
- fusion physique des catalogues local et distant ;
- synchronisation serveur des chemins ou de l'inventaire local ;
- biographie, images d'artiste ou enrichissement provenant de services tiers.
