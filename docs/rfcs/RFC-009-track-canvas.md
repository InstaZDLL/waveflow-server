# RFC-009 — Le canvas d'une piste

- **Statut** : Proposed
- **Implémentée par** : [#155](https://github.com/InstaZDLL/waveflow-server/pull/155)
  (le discriminant du ticket), [#156](https://github.com/InstaZDLL/waveflow-server/pull/156)
  (le magasin et le modèle), [#157](https://github.com/InstaZDLL/waveflow-server/pull/157)
  (les six routes), [#158](https://github.com/InstaZDLL/waveflow-server/pull/158)
  (le drapeau propre), [#163](https://github.com/InstaZDLL/waveflow-server/pull/163)
  (le balayage du magasin). Le champ *Statut* ci-dessus ne bascule jamais dans
  ce projet : c'est cette ligne qui dit ce qui tourne.
- **Date** : 2026-08-28
- **Auteurs** : projet WaveFlow
- **Dépend de** : [RFC-002](RFC-002-waveflow-server-v2.md),
  [RFC-007](RFC-007-library-event-stream.md),
  [RFC-008](RFC-008-receiving-a-file.md)
- **Clôt** : la dernière des quatre demandes du desktop, arbitrées le
  2026-08-24

## Problème

Le desktop veut afficher, pendant la lecture, une courte boucle vidéo attachée à
la piste — ce que Spotify appelle un *canvas*. Le serveur n'a rien de tel : il
sert des octets audio et des pochettes extraites des fichiers, et une boucle
vidéo n'est ni l'un ni l'autre.

La demande ressemble à la réception d'un fichier ([RFC-008](RFC-008-receiving-a-file.md))
et elle n'en est pas une. Cette machinerie est attachée à une **bibliothèque** et
produit une **piste** ; un canvas n'est ni l'une ni l'autre. Il se pose sur une
piste qui existe déjà, il ne crée aucune ligne de catalogue, et l'opérateur ne
l'a pas rangé dans sa collection — c'est le serveur qui le range.

La phrase qui gouverne le RFC-008 gouverne aussi celui-ci, en plus petit :
recevoir un canvas, c'est **dépenser le disque de quelqu'un d'autre, de façon
définitive**. Ce qui change, c'est l'échelle — quelques centaines de kilooctets
plutôt qu'un gigaoctet — et l'échelle décide de la moitié des questions
ci-dessous.

## Décision 1 — le canvas est au serveur, pas à la bibliothèque

Les octets vont dans un magasin adressé par contenu, `canvas_dir`, à côté de
`artwork_dir` sous `data/`. Nommés `<empreinte>.<format>`, l'empreinte étant le
BLAKE3 des octets reçus, comme partout ailleurs dans le projet.

**Pas la racine de la bibliothèque.** Ce n'est pas de l'audio, et surtout
l'opérateur ne l'y a pas mis. La racine est sa collection : ce qu'il sauvegarde,
ce qu'il déplace, ce qu'il compte. Y écrire un objet produit par le serveur
change ce que « supprimer la bibliothèque » veut dire, pour un fichier que le
scanner ignore de toute façon — `discover_audio` filtre sur l'extension.

**Pas `artwork_dir` non plus.** La table `artwork` contraint le format à un jeu
d'images et `read_artwork` en tient la carte MIME ; ranger une vidéo dans le même
répertoire obligerait ces deux listes à rester d'accord pour toujours. Un magasin
séparé n'a pas ce problème, et il peut vivre sur un autre disque.

L'adressage par contenu donne la déduplication sans rien écrire pour elle : les
douze pistes d'un album qui partagent une boucle partagent les octets, et douze
lignes les nomment. C'est la forme juste — un canvas est par piste, comme la
demande le formule, mais il est presque toujours par album dans les faits.

**Une table `canvas` décrit le blob**, `hash` en clé primaire, plus `format`,
`byte_size` et `created_at` : le même quatuor que la table `artwork`, pour la
même raison. Le nom sur le disque est `<empreinte>.<format>`, et une route qui ne
reçoit que l'empreinte ne peut pas le reconstituer — `read_artwork` prend déjà
les deux, parce que la base les lui donne. Deviner en essayant les extensions de
la liste blanche l'une après l'autre serait un balayage du système de fichiers à
chaque requête, pour une réponse que la base tient. La table le tient donc aussi
ici, et c'est elle qui rend le comptage de la décision 5 et le balayage de la
décision 6 possibles : sans elle, le nombre de références d'un blob n'est écrit
nulle part.

## Décision 2 — il se pose à côté de la piste, jamais dessus

Une table `track_canvas`, clé primaire `track_id`, `ON DELETE CASCADE`, portant
l'empreinte du blob et le `library_id` que la tenancy exige. Pas une colonne de
`track`.

Ce `library_id` est **dérivé de la piste**, lu dans la même transaction que
l'écriture, jamais accepté de la requête — c'est ce que fait déjà
`track_override`, dont la route relit `t.library_id` avant d'insérer. Une clé
étrangère composite vers `track(id, library_id)` dirait la même chose au schéma,
mais elle demanderait un index unique sur `track(id, library_id)` que rien
d'autre ne justifie, et la contrainte réelle — que personne ne choisisse la
bibliothèque à laquelle il rattache une piste — est déjà tenue par la lecture.

Le canvas est **fourni par un humain** : rien dans le fichier ne le produit, donc
aucun scan ne peut le retrouver. C'est exactement la propriété qui a donné
`track_override` sa table, et c'est le bon modèle. La pochette est le modèle
inverse et ne doit pas être copié : le scanner l'extrait, et un rescan qui trouve
une image incrustée déplace `track.artwork_hash`. Un lien de canvas posé sur
`track` vivrait dans la liste `ON CONFLICT DO UPDATE SET` de
`apply_catalog_track_in_transaction`, où il faudrait se souvenir de ne pas y
toucher — jusqu'au jour où quelqu'un ajoute une colonne à cette liste et efface
en silence tous les canvas de la bibliothèque.

**Pas une colonne de `track_override` non plus.** Cette table porte des
corrections de *tags*, toutes colonnes de `track`, écrites par
`PATCH /api/v2/tracks/{id}`. Un canvas porte un objet du système de fichiers avec
son cycle de vie propre. L'y loger ferait de la route de métadonnées la
propriétaire silencieuse d'un magasin d'octets, et rendrait « pas de canvas »
indistinguable de « pas de correction » : deux états qui se ressemblent tant
qu'on regarde une valeur nulle. Une table dédiée fait de l'existence de la ligne
le fait lui-même.

## Décision 3 — le ticket dit ce qu'il ouvre

`<video src>` ne peut pas porter d'en-tête `Authorization`. C'est le problème que
[`src/stream_ticket.rs`](../../src/stream_ticket.rs) a déjà résolu pour l'audio,
et il faut le résoudre une seconde fois sans écrire un second module.

La charge du ticket est aujourd'hui figée à 40 octets, `user (16) || track (16)
|| expiry (8)`, et `verify` refuse toute autre longueur. Deux voies : une forme
de ticket propre au canvas, ou un discriminant.

**Un discriminant.** La charge devient `kind (1) || user (16) || track (16) ||
expiry (8)`, et `verify` rend le genre avec la paire ; chaque route affirme celui
qu'elle attend.

Les deux valeurs sont figées ici : **`0x01` pour l'audio, `0x02` pour le
canvas**. `0x00` n'en est pas une, et c'est délibéré — un octet nul est ce qu'une
charge tronquée ou mal remplie produit le plus volontiers, et le genre ne doit
pas être ce que l'erreur choisit. `mint` n'accepte que ces deux valeurs et
`verify` refuse toute autre, du même refus indistinct que la longueur fausse ou
le scellé rompu. Sans ces constantes écrites, deux implémentations lisent le même
ticket différemment et l'une des deux ouvre la mauvaise ressource, ce que le
discriminant existe précisément pour empêcher. Les vecteurs de test couvrent les
deux sens : un aller-retour par genre, et un genre inconnu **scellé sous la bonne
clé**, qui doit être refusé — un ticket forgé ne prouverait rien de ce contrat,
puisqu'il échoue déjà au déchiffrement.

Ce n'est pas de l'économie de code. Sans le discriminant, **un ticket émis pour
un canvas ouvre l'audio**, et réciproquement : deux ressources dont l'une coûte
mille fois l'autre en bande passante, derrière la même autorisation. Le client
n'a jamais demandé ce privilège et l'opérateur ne l'a jamais accordé. Un second
module aurait la même propriété par construction, mais au prix de deux
implémentations du scellé AEAD et de la discipline « toute défaillance se
ressemble » — et deux implémentations divergent.

Les règles de l'audio s'appliquent telles quelles : l'appartenance est
revérifiée à **chaque** rédemption, donc un membre retiré perd le canvas
immédiatement, et le TTL borne la durée d'utilité d'une URL fuitée, pas celle de
l'accès.

Quatre routes, et la raison de chacune :

| Route | Pour qui |
| --- | --- |
| `GET /api/v2/canvas/{empreinte}` | authentifiée ; immuable, `ETag` = empreinte |
| `GET /api/v2/tracks/{id}/canvas` | l'alias, revalidable — il résout le lien courant |
| `POST /api/v2/tracks/{id}/canvas-ticket` | frappe le ticket |
| `GET /api/v2/canvas-stream/{ticket}` | ce que `<video src>` met dans son attribut |

La distinction de cache est celle que la pochette a déjà tranchée en PR #143 :
adressée par empreinte, l'URL ne peut jamais répondre autrement ; adressée par
piste, elle résout le lien du moment et doit rester revalidable. `private` sur
les deux, pour la même raison qu'alors — deux comptes dont les bibliothèques
tiennent le même canvas en partagent l'empreinte.

**L'empreinte n'est pas une autorisation.** C'est le corollaire de la phrase
précédente et il vaut d'être écrit : puisque deux comptes étrangers l'un à
l'autre peuvent tenir la même, connaître une empreinte n'établit rien.
`GET /api/v2/canvas/{empreinte}` la résout donc comme `artwork_for_user` résout
une pochette — en exigeant qu'une bibliothèque dont le demandeur est membre la
référence — et répond 404 sinon. Ni `private` ni l'`ETag` ne remplacent ce
contrôle : le premier parle des caches intermédiaires, le second de la fraîcheur.
Une empreinte que personne d'accessible ne référence et une qui n'existe pas
répondent la même chose, comme partout ailleurs dans cette API.

**L'alias n'en est pas dispensé.** `GET /api/v2/tracks/{id}/canvas` s'adresse par
un identifiant de piste, qui se devine aussi mal et se partage aussi bien qu'une
empreinte : connaître l'un n'établit pas plus que connaître l'autre. La route
exige donc l'appartenance à la bibliothèque de la piste, relue **à chaque
requête** et pas seulement à la première, et répond 404 dans les trois cas qui
doivent se ressembler — la piste n'existe pas, elle appartient à quelqu'un
d'autre, elle n'a pas de canvas. C'est la règle générale du projet, la tenancy
tenue dans les requêtes et `Forbidden` projeté sur 404 ; elle se répète ici parce
qu'une route qui *résout* un lien avant de servir des octets est exactement
l'endroit où l'on autorise le lien et où l'on oublie la piste.

`canvas-stream` et non `canvas/{ticket}` : un segment littéral sous le même
préfixe qu'un paramètre est un piège pour le prochain lecteur, même quand le
routeur tranche correctement.

## Décision 4 — il arrive en une fois, et le serveur seul le nomme

`PUT /api/v2/tracks/{id}/canvas`, corps unique, et `DELETE` sur la même route
pour l'enlever.

Pas de négociation, pas de fragments, pas de session. Tout cet appareil existe
dans le RFC-008 parce qu'un fichier de musique peut faire un gigaoctet et qu'un
transfert doit pouvoir reprendre. Un canvas tient dans une requête. Lui imposer
une machine de reprise coûterait le triple des surfaces pour un cas qui ne se
présente pas.

La route porte **son propre plafond de corps**, dérivé de
`WAVEFLOW_CANVAS_MAX_BYTES`, comme les routes de téléversement portent le leur.
La limite globale de 16 Kio ne bouge pas : une API dont toutes les autres routes
acceptent cette taille ne peut pas être noyée par un corps de requête.

**Le serveur calcule l'empreinte, le client n'en annonce aucune.** La décision 3
du RFC-008 tient ici sous une forme plus simple : là-bas, l'empreinte annoncée
servait à *éviter un transfert*, jamais à établir une identité ; ici il n'y a
aucun transfert à éviter, donc aucune raison d'accepter une annonce.

Le format est vérifié en **sondant les octets reçus**, jamais en croyant un
`Content-Type` ou une extension. `ffprobe` est déjà une dépendance et déjà sur le
`PATH` de tout déploiement. Sont exigés : un conteneur de la liste blanche, un
flux vidéo présent, et une durée sous `WAVEFLOW_CANVAS_MAX_DURATION_SECS`. Cette
dernière borne n'est pas de la prudence : sans elle, « une courte boucle » devient
de l'hébergement vidéo, qui est un autre produit avec d'autres coûts.

## Décision 5 — c'est le disque de l'opérateur, et il l'a dit une fois

Le rôle requis est `may_upload` — `Owner` ou `Manager` — et la bibliothèque doit
porter `accepts_canvas`.

Le canvas suit les règles du **dépôt**, pas celles de la correction. La ligne est
là : une correction de tags dépense des octets dans une ligne, un canvas dépense
des octets sur un disque. `may_write_metadata` et `may_upload` nomment la même
paire de rôles aujourd'hui ; la question n'est pas laquelle choisir mais laquelle
suivre le jour où elles divergent, et c'est la seconde.

**Un drapeau propre**, `accepts_canvas`, éteint par défaut et distinct
d'`accepts_uploads`.

Cette RFC avait d'abord tranché l'inverse : un seul drapeau, qui répond « un
membre de cette bibliothèque peut-il dépenser le disque de l'opérateur », et les
deux réponses vont ensemble *tant que personne n'a montré un cas où elles se
séparent*. La condition était explicite, et le cas est venu — du desktop, le
2026-08-30. Un serveur en lecture seule est l'installation la plus courante qui
soit, et un opérateur peut parfaitement vouloir quelques centaines de kilooctets
de boucle sur une machine dont il refuse absolument qu'elle grossisse en audio.
Lier les deux rendait la demande (d) inerte exactement là où elle est le plus
attendue — un coût que la version précédente nommait dans ses questions ouvertes
en le croyant théorique.

Le rôle ne bouge pas : `may_upload`, la paire `Owner | Manager`. C'est le
*quoi* qui se sépare, pas le *qui*.

Éteint par défaut, et surtout **pas hérité d'`accepts_uploads`** : une
bibliothèque qui prend déjà des fichiers n'a rien dit sur les boucles, et
l'héritage ferait souscrire l'opérateur à quelque chose qu'il n'a pas nommé.

**Un quota propre**, `WAVEFLOW_CANVAS_LIBRARY_QUOTA_BYTES`, distinct de celui des
téléversements. Les mélanger laisserait les boucles affamer la place que le quota
existe pour protéger — celle de la musique — et les deux magasins peuvent vivre
sur des disques différents.

Il compte **les blobs distincts qu'une bibliothèque référence**, pas ses liens :
le canvas partagé par les douze pistes d'un album est facturé une fois, ce qui
est exactement ce que la déduplication de la décision 1 économise. Facturer les
liens rendrait le prix d'un canvas dépendant du nombre de pistes auxquelles on
l'attache, et un membre paierait douze fois des octets écrits une seule.

Un blob que **deux bibliothèques** référencent est compté par chacune, bien qu'il
n'existe qu'une fois sur le disque. C'est délibéré : le plafond d'une
bibliothèque ne doit pas dépendre de ce qu'une autre se trouve tenir, sans quoi
la première à poser un canvas le paierait et la seconde l'obtiendrait pour rien,
jusqu'à ce que la première l'enlève et voie sa voisine hériter de la facture.
L'opérateur y perd une somme qui surestime son disque ; il n'y perd pas la
capacité de prévoir ce qu'une bibliothèque peut coûter.

**Rien à réserver.** La réservation du RFC-008 existe parce qu'une négociation
est une promesse ouverte que rien ne tient jusqu'à la validation, et que deux
promesses simultanées franchissent un contrôle qui ne compte encore rien. Ici il
n'y a pas de promesse : le corps est déjà borné par le plafond de la route, donc
le quota se vérifie et se dépense dans la même transaction, sous le verrou
d'écriture. Pas de promesse, pas de course.

## Décision 6 — l'ordre d'écriture, et les deux pannes ne se valent pas

**Le fichier d'abord, la ligne ensuite. Puis, à la suppression : la ligne
d'abord, le fichier ensuite.** Les deux ordres laissent la même panne, et c'est
la panne récupérable.

Le magasin est adressé par contenu, donc énumérable : un balayage peut lister
`canvas_dir` et demander à la base quelles empreintes sont encore référencées.
Des octets orphelins se retrouvent. Une ligne qui nomme un fichier absent, elle,
ne se retrouve pas — c'est un lien mort que le client voit.

**Et le fichier ne part qu'avec la dernière référence.** Retirer son canvas à une
piste retire une ligne de `track_canvas`, pas un blob : onze autres pistes
peuvent le nommer. La suppression de la ligne, le décompte des références
restantes et la décision d'effacer les octets tiennent donc dans **une seule
transaction**, sous le verrou d'écriture — c'est ce qui empêche deux retraits
simultanés de conclure chacun qu'il reste une référence et de laisser un blob que
plus rien ne nomme. Le quota se rend au même moment, et pour la bibliothèque qui
a perdu sa dernière référence seulement.

**Les octets ne partent qu'après le commit.** La transaction décide, l'effacement
suit une fois la base engagée. L'ordre inverse — effacer puis commettre — laisse
une ligne qui nomme un fichier absent dès que la transaction échoue, c'est-à-dire
exactement le lien mort que tout cet ordre existe pour éviter. La panne entre les
deux laisse au contraire des octets que plus rien ne nomme, et elle se rattrape :
c'est la reprise, et c'est ce pour quoi le magasin est énumérable.

**Mais un verrou couvre le cycle entier.** Commettre puis effacer ouvre une
fenêtre, et elle n'est pas théorique : entre les deux, un `PUT` du même contenu
ne trouve plus aucune ligne, écrit ses octets, insère la sienne — et l'effacement
qui suit emporte le fichier d'un lien vivant. C'est le lien mort à nouveau,
atteint par l'autre bout. Et tenir la barrière d'écriture plus longtemps ne
suffirait pas : la pose écrit le fichier *avant* sa ligne, donc une écriture
faufilée avant la barrière et une ligne insérée après elle rejoindraient la même
fenêtre par le côté.

**Un verrou par empreinte**, et non la barrière d'écriture. La pose le prend
avant d'écrire les octets et le rend après avoir inséré la ligne ; le retrait le
prend avant sa transaction et le rend après l'effacement. Poser un canvas et en
retirer un ne se chevauchent donc jamais — et deux empreintes différentes ne se
gênent pas, ce que la barrière globale leur imposerait sans rien y gagner : la
course est par blob.

Ce n'est pas seulement plus fin, c'est la seule forme disponible. La barrière
d'écriture est **process-wide** et ne doit jamais couvrir une E/S fichier — c'est
la règle que `upload_locks` suit déjà, écrite dans `src/services/mod.rs` : « file
I/O has no business happening while the process-wide gate is held ». Une version
antérieure de cette décision demandait le contraire ; elle avait été écrite sans
cette règle sous les yeux. La barrière reste donc ce qu'elle est partout
ailleurs, l'enveloppe d'une transaction SQL, et le verrou par empreinte porte le
reste.

Le sondage `ffprobe` de la décision 4 reste dehors des deux : il ne touche à
rien, et attendre un sous-processus sous un verrou est ce qu'on ne fait pas.

Le RFC-008 avait la même course et n'a pas pu la fermer : `commit_upload`
constate qu'une autre session sur la même empreinte peut être en train de
commettre entre son rename et sa transaction, et un fichier d'un gigaoctet ne se
sérialise pas derrière un verrou. Il l'a donc *gardée*, avec des sessions, un nom
de travail et un balayage sous garde. Ici l'échelle permet de la fermer.

**Et le schéma ferme la même porte une seconde fois.** `track_canvas.canvas_hash`
référence `canvas(hash)` sans clause `ON DELETE`, donc SQLite refuse de supprimer
la ligne d'un blob qu'un lien nomme encore. Le décompte de la transaction et
cette contrainte disent la même chose ; la contrainte est celle qui parle en
dernier, et c'est bien qu'elle existe — un décompte est du code, une contrainte
est une propriété.

**Et la ligne `canvas` part avec la dernière référence.** Elle décrit un blob ;
quand plus rien ne nomme le blob, elle ment. Elle est donc supprimée dans la même
transaction que la dernière ligne `track_canvas` — donc avant les octets, comme
la ligne de lien et pour la même raison — et jamais marquée : un état « décrit un
fichier disparu » se propagerait dans toutes les lectures pour n'être utile à
aucune.

**La cascade contourne tout cela**, et cela vaut d'être écrit plutôt que
découvert. `track_canvas` cascade depuis `track`, qui cascade depuis `library` :
supprimer une piste ou une bibliothèque effacerait les liens sans décompter les
références, sans rendre le quota et sans toucher aux octets. Rien n'emprunte ce
chemin aujourd'hui — aucune route ni commande ne supprime une piste ou une
bibliothèque, une piste absente est *marquée indisponible* — donc c'est un piège
posé pour plus tard et non un défaut actuel. Le jour où l'une des deux
suppressions existe, elle passe par la transaction de la décision 6 ou elle
laisse derrière elle un magasin que seul le balayage rattrape. `artwork` a déjà
exactement cette propriété et personne ne l'avait écrite.

C'est l'inverse de l'ordre du RFC-008, et sans contradiction : là-bas la ligne de
session **était** le seul souvenir qu'un fichier de travail existait, sous un nom
que rien n'énumère. Ici c'est le contenu qui se souvient de lui-même.

## Décision 7 — rien à annoncer, sauf le lien

Le [RFC-007](RFC-007-library-event-stream.md) a déjà tranché, et cette RFC ne
fait que s'y conformer : pas d'événement pour une ressource adressée par contenu.
Les octets derrière une empreinte ne changent jamais. Seul le **lien** d'une
piste vers un canvas est un changement, et il voyage dans l'`upsert` de la piste.

Poser ou retirer un canvas émet donc un événement `track` / `upsert` par le même
chemin `LibraryChange` qu'une correction de métadonnées, avec l'appareil
d'origine que la PR #152 vient d'ajouter — sans quoi le client qui vient de poser
le canvas le recevrait comme une découverte. Rien de neuf n'est nécessaire, et
c'est la mesure que le RFC-007 a été construit au bon endroit.

**La charge de l'événement ne change pas non plus.** Elle porte aujourd'hui
`{ "full_hash": … }` sur un `upsert`, et rien d'autre : le condensé y est parce
que **rien d'autre ne le porte** — une piste garde son identifiant pendant que
ses octets bougent, et l'événement est le seul témoin. Un lien de canvas n'est
pas dans ce cas ; il se lit sur la piste, que le client relit de toute façon en
recevant l'événement. Y ajouter un champ, et un `canvas: null` pour dire qu'il a
disparu, ferait de la charge une projection partielle de la piste — un second
modèle à tenir d'accord avec le premier, pour une information déjà disponible à
un aller-retour de là.

## Décision 8 — ce que le canvas ne fait pas

- **Il n'est pas transcodé.** Le serveur stocke et sert ce qu'on lui donne. La
  chaîne FFmpeg est celle de l'audio ; l'y attacher ferait du canvas une charge
  de calcul par lecture, pour une boucle de quelques secondes.
- **Il n'est pas extrait, ni engendré.** Aucun fichier ne le contient et le
  serveur ne va rien chercher dehors — le refus d'enrichissement du RFC-007
  s'applique tel quel.
- **Il n'apparaît pas dans la façade Subsonic.** Le contrat est gelé pour
  `v2.0-beta`, et aucun client validé ne le demande.
- **Il ne voyage pas dans un partage.** `/share/{token}` sert de l'audio à un
  visiteur sans compte ; y ajouter une surface se décide séparément, pas en
  passant.

## Décision 9 — le son part à l'ingestion

Ouvert par la première version de cette RFC, qui posait trois options sans en
choisir : refuser un flux audio, l'ignorer à la lecture, ou laisser le client
décider. Tranché le 2026-08-30, et il fallait le trancher parce que le code
l'avait déjà tranché **par omission** — la sonde exigeait un flux vidéo et ne
disait rien de l'audio, donc un canvas sonore était accepté sans que personne
l'ait décidé.

**La troisième option n'existe pas.** Le desktop rend son `<video>` avec `muted`
en dur, plus `aria-hidden` et `pointer-events-none` : ce n'est pas un réglage,
c'est ce qu'un canvas est — une boucle qui tourne par-dessus une piste déjà en
cours de lecture. Aucun client ne peut décider d'entendre ce son sans parler
par-dessus la musique. « Laisser le client décider » revenait donc à stocker et
servir des octets que personne n'entendra jamais.

Et ces octets coûtent. Ils sont pris sur les 4 Mio du plafond, au détriment de
l'image que les mêmes octets auraient pu payer.

**Le vrai choix est donc entre refuser à la sonde et accepter en retirant la
piste.** C'est la seconde. Refuser rejetterait un mp4 par ailleurs valable pour
un flux que celui qui l'envoie n'a pas forcément choisi d'inclure — beaucoup
d'encodeurs écrivent une piste vide par défaut — et pour un motif que
l'opérateur ne peut pas diagnostiquer en regardant sa vidéo.

**C'est un remuxage, pas un ré-encodage**, et la décision 8 n'y fait pas
obstacle. Son argument était qu'un transcodage ferait du canvas une charge de
calcul *par lecture* ; ici c'est une copie de flux, une fois, à l'ingestion, et
l'image ressort bit pour bit ce qu'elle était.

**L'empreinte se calcule après.** Le magasin est adressé par contenu, donc deux
membres offrant le même fichier doivent produire la même sortie, sans quoi la
déduplication de la décision 1 cesse silencieusement de fonctionner et chacun
paie son blob. Le remuxage est donc rendu reproductible — métadonnées écartées,
sortie *bitexact*. Cela tient à l'intérieur d'un déploiement ; deux versions de
FFmpeg peuvent encore muxer les mêmes paquets différemment, si bien qu'une mise
à jour du serveur peut donner un second blob au même fichier source. Cela coûte
un doublon, jamais une réponse fausse.

## Décision 10 — qui ramasse les octets qu'une panne a laissés

La décision 6 dit quand un blob cesse d'être référencé et par quelle
transaction ; elle laissait ouvert *qui* ramasse ce qu'une panne a laissé
derrière. Un balayeur, dans la forme de celui du RFC-008 : une passe au
démarrage, puis une par jour. Les orphelins sont faits par des pannes et non par
l'usage, donc c'est une réparation et pas de l'entretien — sur un serveur qui
n'a pas planté, la passe ne trouve rien et coûte une lecture de répertoire.

**Trois sortes de fichiers, et une seule est supprimée sans réserve.**

Un blob que plus aucune ligne `canvas` ne nomme part — mais sous le verrou de
son empreinte, et le compte est relu à l'intérieur. Sans ce verrou, ce balayeur
serait la cause du dommage qu'il existe pour réparer : une pose écrit son
fichier, et entre cette écriture et sa ligne le balayeur voit des octets que
rien ne nomme et les prend, laissant la pose commettre une ligne dont le fichier
a disparu.

Un fichier de travail — `<uuid>.part`, `<uuid>.silent` — part s'il est plus
vieux qu'une heure. Ces noms n'appartiennent qu'à l'appel qui les a écrits :
aucune ligne ne les mentionne et il n'existe pas de verrou à prendre pour eux,
donc l'âge tient lieu des deux. Une heure est très au-delà de ce qu'une pose
vivante peut encore être en train de faire.

Le radical est vérifié aussi strictement que celui d'un blob : c'est un UUID que
ce module a inventé, et l'exiger ne coûte rien. Sans cette exigence,
l'`avancement.part` de l'opérateur serait la seule chose du magasin à porter une
extension qui suffit à la faire disparaître — une asymétrie qui contredirait la
règle du paragraphe suivant au moment même où on l'énonce.

**Et tout le reste est compté, jamais supprimé.** Le magasin vit sous le `data/`
de l'opérateur. Un balayeur qui efface ce qu'il ne sait pas nommer est un
balayeur que personne ne devrait lancer : un fichier n'est candidat que s'il est
`<64 hex>.<format>` pour un format de la liste blanche, ou l'un des deux noms de
travail que ce module écrit.

**La ligne sans fichier est signalée et jamais réparée.** C'est la panne que la
décision 6 appelle irrécupérable — un lien mort que le client voit — et la
« réparation » consisterait à supprimer le canvas de quelqu'un. Répondre à un
lien mort en le rendant absent est une plus mauvaise réponse que de dire la
vérité à l'opérateur.

## Décision 11 — les octets se partagent, la connaissance non

La décision 5 fixe le prix d'un blob que deux bibliothèques tiennent — chacune
le compte — sans dire ce que la frontière signifie. La voici, et l'essentiel est
qu'elle décrit ce que le code fait déjà plutôt que ce qu'il devrait faire.

**Le fichier est partagé, l'existence de l'empreinte ne l'est pas.** Une seule
copie sur le disque, et `canvas_for_user` n'y donne accès qu'à travers une
bibliothèque dont le demandeur est membre. Une empreinte que personne
d'accessible ne référence et une empreinte qui n'existe pas répondent la même
chose. Le RFC-008 avait déjà tranché la question analogue et pour la raison qui
vaut ici : une déduplication *visible* entre bibliothèques serait l'oracle que
la règle du 404 interdit — elle dirait à B qu'un fichier qu'elle ne possède pas
existe quelque part.

C'est pourquoi chacune paie. Ce n'est pas une maladresse comptable : facturer au
premier déposant ferait dépendre le plafond d'une bibliothèque de ce qu'une
autre se trouve tenir, et le jour où la première retire son canvas, sa voisine
hériterait d'une facture qu'elle n'a pas vue venir. L'opérateur y perd une somme
qui surestime son disque ; il n'y perd pas la capacité de prévoir ce qu'une
bibliothèque peut coûter.

**Ce qui reste hors de cette RFC** est ce que le RFC-008 en avait déjà écarté :
rattacher, lier matériellement ou copier un blob d'une bibliothèque vers une
autre. Ce sont des opérations qui *déplacent une frontière* plutôt que de la
respecter, et aucune n'est demandée. Elles ne sont pas exclues pour toujours ;
elles demandent de décider ce qu'une bibliothèque a le droit d'apprendre d'une
autre, et ce n'est pas une question que le canvas doit trancher pour tout le
monde.

## Ce que cette RFC change ailleurs

**La charge du ticket de flux grandit d'un octet**, et les tickets frappés avant
la mise à jour cessent de valider — `verify` contrôle la longueur exacte. Le
dommage est borné par le TTL : un navigateur en cours de recherche reçoit un 404
et refrappe. Cela mérite d'être dit plutôt que découvert, mais ne mérite pas un
champ de version : le format est scellé sous la clé d'instance et n'est jamais
persisté.

**`Config` gagne un répertoire et trois bornes.** `canvas_dir`,
`WAVEFLOW_CANVAS_MAX_BYTES`, `WAVEFLOW_CANVAS_MAX_DURATION_SECS`,
`WAVEFLOW_CANVAS_LIBRARY_QUOTA_BYTES` — et les valeurs qui n'ont de sens que
l'une contre l'autre se valident au démarrage, comme `validate_uploads` le fait
déjà.

**Les valeurs retenues**, choisies en implémentant et livrées par la PR #156 :

| Réglage | Défaut | Pourquoi celui-là |
| --- | --- | --- |
| `WAVEFLOW_CANVAS_MAX_BYTES` | 4 Mio | Une vraie boucle pèse quelques centaines de kilooctets. Confortablement au-dessus de tout ce qui est encore une boucle, assez bas pour que le plafond de corps dérivé reste un plafond. |
| `WAVEFLOW_CANVAS_MAX_DURATION_SECS` | 15 | Ces boucles durent trois à huit secondes. De la place pour une inhabituelle, pas pour un épisode. |
| `WAVEFLOW_CANVAS_LIBRARY_QUOTA_BYTES` | 1 Gio | Des milliers de canvas réels, et un nombre qu'un opérateur peut tenir contre un disque. |
| Liste blanche des conteneurs | `mp4`, `webm` | Les deux que navigateur et bureau lisent sans décodeur à embarquer. Courte exprès : chaque entrée est un format que le serveur promet de servir pour toujours, puisque les octets derrière une empreinte ne changent jamais. |

`canvas_dir` se dérive de `WAVEFLOW_DATA_DIR` plutôt que de se régler seul,
exactement comme `artwork_dir`.

## Ce qui reste ouvert

Aucune question de forme ne reste ouverte : la décision 11 ferme la dernière.
Ce qui suit est du travail en attente, pas un arbitrage.

- **Le balayage de `artwork_dir`.** La décision 10 ramasse les octets du
  magasin de canvas ; la pochette a exactement la même propriété depuis le
  début et reste sans balayeur. Le même travail sur un autre magasin, avec des
  références réparties sur trois colonnes au lieu d'une — donc pas la même
  prudence, et pas le même jour que du code qui supprime des fichiers.

