# RFC-008 — Réception d'un fichier

- **Statut** : Proposed
- **Date** : 2026-08-26
- **Auteurs** : projet WaveFlow
- **Dépend de** : [RFC-002](RFC-002-waveflow-server-v2.md),
  [RFC-007](RFC-007-library-event-stream.md)
- **Rend possible** : l'équilibrage des deux sources côté desktop, et l'asset
  vidéo par piste

## Problème

Rien dans `/api/v2` n'accepte d'audio entrant. Le serveur est une source ; il
n'est pas un dépôt. Le desktop veut téléverser automatiquement les morceaux qui
manquent au serveur, ce qui suppose exactement l'inverse.

C'est la plus lourde des quatre demandes qu'il a formulées, et pas parce qu'elle
demande beaucoup de code. Elle demande peu. Elle change ce qu'est le serveur :
recevoir un fichier, c'est **dépenser le disque de quelqu'un d'autre, de façon
définitive**. Presque toutes les décisions ci-dessous découlent de cette phrase.

## Décision 1 — une bibliothèque opte, elle ne subit pas

Deux verrous, et ils répondent à deux questions différentes.

Le rôle dit **qui** : `Owner` ou `Manager`, la même paire que
[`may_scan`](../../src/database.rs) et que la correction de métadonnées, parce
qu'un membre qui peut déjà dépenser le disque du propriétaire sur un rescan est
le même niveau de confiance.

Un drapeau `library.accepts_uploads`, faux par défaut, dit **où**. Un serveur
existant ne devient pas un dépôt parce qu'il a été mis à jour ; son opérateur le
décide bibliothèque par bibliothèque. Sans ce drapeau, le rôle suffirait à faire
d'une installation de lecture seule une installation qui grossit — un changement
que personne n'aurait demandé.

Un rôle `Uploader` distinct — un membre qui téléverse sans pouvoir rescanner —
est concevable et n'est pas inventé ici. Les deux verrous existants suffisent à
commencer ; un troisième se décide quand un usage le réclame, pas avant.

Les deux verrous sont réévalués **à chaque étape** : la négociation, chaque
fragment, la validation. Une session n'est pas une autorisation qu'on emporte.

Le serveur a déjà cette règle ailleurs, et pour la même raison : un ticket de
lecture rejoue la vérification d'appartenance à chaque redemption, « so revoking
access takes effect immediately instead of lasting until the ticket expires ».
Une session de téléversement dure bien plus longtemps qu'un ticket et coûte
bien plus cher : un membre exclu, ou une bibliothèque refermée par son
opérateur, doit cesser d'écrire à la requête suivante — pas à la fin d'un
transfert commencé avant. Le fragment est refusé, la validation aussi, la zone
de travail est nettoyée et la réservation rendue ; le refus prend la forme du
verdict correspondant, ou un 404 quand c'est l'appartenance elle-même qui a
disparu.

## Décision 2 — l'empreinte avant les octets

Le téléversement se négocie avant de transférer quoi que ce soit. Le client
annonce ce qu'il a — `full_hash`, taille, extension — et le serveur répond par un
**verdict nommé**.

| Verdict | Sens | Suite pour le client |
| --- | --- | --- |
| `present` | la bibliothèque visée détient déjà ces octets | rien à transférer |
| `accepted` | session ouverte : identifiant, fragment attendu, expiration | transférer |
| `unsupported_format` | extension hors `AUDIO_EXTENSIONS` | définitif |
| `too_large` | au-dessus du plafond par fichier | définitif |
| `quota_exceeded` | la bibliothèque n'a plus la place | réessayable plus tard |
| `library_closed` | `accepts_uploads` est faux | définitif jusqu'à l'opérateur |
| `too_many_sessions` | plafond de sessions simultanées atteint | réessayable |

Un verdict, jamais un booléen. `known` / `unknown` disait deux choses à la fois —
« je ne l'ai pas » et « vas-y » — et taisait les quatre refus qui doivent
pourtant arriver ici : **c'est le seul moment où un refus est bon marché**, avant
le premier octet plutôt qu'après le dernier. Les nommer est ce qui permet au
client de distinguer ce qu'il ne doit jamais réessayer de ce qu'il peut
réessayer.

Une précision que la table ne peut pas porter : `library_closed` ne concerne
qu'un appelant qui a le droit d'être là. Un non-membre, ou un membre au rôle
insuffisant, reçoit un 404 comme partout ailleurs — le RFC-002 pose que
`Forbidden` s'y confond, et une réponse « fermée » confirmerait l'existence de la
bibliothèque à qui n'y a pas droit.

Les formats acceptés sont ceux que le scanner sait indexer —
`waveflow_core::scanner::AUDIO_EXTENSIONS` et rien d'autre. Accepter un fichier
que le catalogue ne saura pas lire serait accepter d'occuper de l'espace pour
rien.

### `present` veut dire : dans cette bibliothèque

La recherche d'empreinte est bornée à la bibliothèque visée. Deux raisons, et la
seconde n'est pas une préférence.

L'index existe déjà pour ce cas :
`track_library_hash_idx ON track(library_id, full_hash, is_available)`. Chercher
par bibliothèque est la forme que le schéma sert.

Et une recherche globale **confirmerait une existence**. Un membre de la
bibliothèque B qui annonce une empreinte et s'entend répondre `present` alors que
seule A détient ces octets vient d'apprendre quelque chose sur une bibliothèque
qu'il ne peut pas voir. Le RFC-002 pose que 404 confond l'absent et l'étranger ;
une déduplication globale serait exactement l'oracle que cette règle interdit.
Elle laisserait aussi B croire qu'elle possède une piste qu'elle n'a pas.

Le partage d'un même blob entre bibliothèques — rattachement, lien matériel,
copie interne — n'est pas exclu pour toujours, mais il demande de décider ces
frontières et il n'appartient pas à cette RFC.

### La négociation se fait par lot

Cinq mille candidats ne feront pas cinq mille allers-retours. La négociation
prend un tableau d'empreintes et rend un tableau de verdicts.

Avec une conséquence à ne pas manquer : la limite de corps du routeur est de
seize kilo-octets (décision 4), soit deux cents empreintes environ une fois le
JSON compté. Le lot est donc **borné explicitement**, et cette route porte sa
propre limite de corps comme celle des fragments. Un lot trop grand est refusé
pour ce qu'il est, jamais tronqué en silence.

Le desktop signale que le gros de la déduplication se fera chez lui, hors ligne :
son miroir de catalogue recopie déjà le `full_hash` de chaque piste, que la
projection native expose. La négociation sert les cas isolés et les autres
clients, pas le volume — une raison de plus de ne pas sur-construire ici.

### Une session se retrouve, elle ne se duplique pas

Une négociation portant une empreinte pour laquelle une session est déjà ouverte,
pour le même compte et la même bibliothèque, **rend cette session** — pas une
seconde. Sinon chaque redémarrage du client abandonne une zone de travail et,
avec la réservation ci-dessous, immobilise du quota que rien ne libère.

Le nombre de sessions simultanées par compte est plafonné, comme le transcodage
l'est déjà par `WAVEFLOW_TRANSCODE_PER_USER_LIMIT`. Un client à cinq mille
fichiers ouvre autant de sessions qu'il a le droit d'en tenir, pas cinq mille.

### Le quota se réserve à l'ouverture

Vérifier le quota à la négociation sans rien retenir laisserait deux sessions de
quatre gigaoctets franchir le contrôle avec cinq gigaoctets libres, et le
dépassement n'apparaîtrait qu'une fois le disque écrit.

La taille annoncée est donc **réservée** à l'ouverture, rendue à l'expiration ou
à l'échec, et le total revérifié à la validation. Les deux, pas l'un ou l'autre :
la réservation empêche la course, la revérification rattrape ce que la
réservation avait seulement supposé.

Une validation réussie ne rend pas la réservation : elle la **convertit**, dans
la même transaction que la piste. Rendre puis recompter laisserait un intervalle
pendant lequel l'espace est libre des deux côtés — le fichier est sur le disque
et le quota ne le compte pas encore — et deux validations simultanées passeraient
par ce trou. La réservation cesse d'être une promesse au moment où elle devient
une dépense, sans jamais cesser d'être l'une ou l'autre.

## Décision 3 — l'empreinte annoncée n'établit jamais rien

Le serveur recalcule le `full_hash` à la validation et refuse si les deux
diffèrent, en jetant ce qui a été reçu.

L'empreinte annoncée sert à **éviter un transfert**, jamais à **établir une
identité**. La distinction n'est pas théorique : une identité fondée sur ce que
le client affirme laisserait n'importe quel membre autorisé faire passer un
fichier pour un autre, et la déduplication de la décision 2 deviendrait un moyen
de substitution plutôt qu'une économie.

La taille annoncée est bornée de la même façon, mais plus tôt : le serveur compte
ce qu'il reçoit et interrompt la session dès que le compte dépasse la taille
annoncée, sans attendre l'empreinte.

Ce n'est pas un second contrôle d'identité — une empreinte qui correspond
garantit déjà la taille, ce sont les mêmes octets. C'est ce qui empêche un client
qui ment d'écrire quarante gigaoctets avant que l'empreinte ait l'occasion de le
prendre en défaut.

## Décision 4 — fragmenté, et la limite globale ne bouge pas

Le routeur pose `DefaultBodyLimit::max(16 * 1024)`. Ce n'est pas un oubli : une
API dont chaque route accepte seize kilo-octets ne peut pas être noyée par un
corps de requête, et cette propriété vaut d'être gardée.

La route de téléversement porte donc **sa propre borne, par fragment**, et ne
relève pas la limite du routeur. Un plafond global de plusieurs mégaoctets
donnerait à chaque route du serveur — y compris celles qui n'attendent qu'un
identifiant — une surface qu'aucune n'a demandée.

Fragmenté aussi parce que le desktop l'a bien posé : plusieurs milliers de
morceaux sur une liaison domestique ne passent pas en une fois.

### Une reprise ne doit pas dépendre d'un accusé reçu

Une session porte son identifiant, la bibliothèque, le compte, l'empreinte et la
taille annoncées, l'extension, les octets reçus, l'index du fragment attendu et
son expiration. Cet état **se lit**, de sorte qu'un client qui redémarre demande
où il en est au lieu de le déduire.

Trois cas à l'arrivée d'un fragment `N`, et un seul est une erreur :

- `N == attendu` — écrit, l'attendu avance.
- `N < attendu` — déjà reçu. Réponse idempotente, avec l'attendu courant.
- `N > attendu` — conflit, avec l'attendu courant.

Le deuxième cas mérite d'être écrit plutôt que subi. Un fragment écrit dont
l'accusé se perd est le cas ordinaire d'une liaison qui coupe, pas une faute du
client ; le traiter comme un rejet rendrait le protocole fragile là où il devait
justement absorber les coupures. Le troisième est refusé pour la raison inverse :
un fragment sauté laisse un trou que seule l'empreinte finale révélerait,
beaucoup trop tard.

## Décision 5 — le serveur nomme le fichier, et il le nomme par son empreinte

Le client ne propose jamais de chemin, pas même un nom.

Toute une famille de problèmes disparaît avec cette phrase : la traversée de
répertoire, l'écrasement d'un fichier que l'opérateur avait rangé lui-même, la
collision entre deux téléversements simultanés. Aucun de ces problèmes n'a besoin
d'être résolu s'il n'est jamais posé.

Le fichier atterrit dans un sous-répertoire que le serveur possède, **à
l'intérieur de la racine de la bibliothèque**, pour que le scan ordinaire le
trouve sans qu'on lui apprenne un second endroit où regarder. Jamais un lien
symbolique — le parcours les refuse déjà, délibérément.

Reste sous quel nom, et la question a une réponse : **par empreinte**, pas par
tags.

Ranger par artiste / album / titre donne une arborescence qu'un opérateur
reconnaît, et c'est le seul argument en sa faveur. Contre lui : les tags bougent.
Une correction de métadonnées, une reprise du RFC-006, un retag par l'opérateur
change le nom que le chemin aurait figé — et le serveur se retrouve devant un
choix qu'il n'avait aucune raison de poser : déplacer le fichier, ou garder un
chemin qui ment.

Le projet a déjà tranché cette question dans l'autre sens, ailleurs. Une
correction de métadonnées ne réécrit jamais le fichier ; elle vit à côté de lui.
**Le fichier ne bouge pas.** Ranger par tags réintroduirait exactement le
couplage que cette décision a écarté, du côté du stockage cette fois — un
catalogue logique qui commande une organisation physique.

Par empreinte, mais **avec son extension** : `<full_hash>.<extension>`, prise
dans l'ensemble validé à la négociation. Ce n'est pas de la cosmétique. Le
parcours ne reconnaît un fichier que par son extension — c'est la même règle qui
rend un fragment `.part` invisible à la décision 6 — donc un fichier nommé de sa
seule empreinte serait invisible lui aussi, et le rattrapage de la décision 7,
qui compte sur le prochain scan pour ramasser un orphelin, ne ramasserait rien.
Les deux moitiés de cette règle vont ensemble : ce qui est incomplet n'a pas
d'extension, ce qui est complet en a une.

L'arborescence lisible n'est pas perdue : elle redevient ce qu'elle est, une vue
— une exportation, si elle est demandée un jour — et non la représentation
interne.

## Décision 6 — rien de partiel n'est jamais visible

Les fragments s'écrivent **dans le répertoire de destination lui-même**, sous un
nom qui ne porte pas d'extension audio : `<uuid>.part` jusqu'au renommage final.

La première version de cette RFC plaçait la zone de travail hors de la racine, ce
qui se contredisait : une zone temporaire système peut être sur un autre montage,
et `rename()` cesse alors d'être atomique — il devient une copie suivie d'un
effacement, avec une fenêtre pendant laquelle un fichier tronqué existe. Écrire
au même endroit que la destination donne le même système de fichiers
gratuitement, et il n'y a plus de déplacement entre montages à espérer.

Restait l'invisibilité, et elle vient d'une règle qui existe déjà. Le parcours ne
saute aucun répertoire — `WalkDir::new(root).follow_links(false)`, sans
`filter_entry` : un répertoire caché serait bel et bien visité, et compter sur le
point de tête aurait été une illusion. Ce que le parcours filtre, c'est
l'extension. Un fragment nommé `.part` est donc inindexable **par construction**,
sans qu'on ait à apprendre au scanner un chemin à éviter — une règle
supplémentaire qu'une refonte du parcours pourrait casser en silence.

Deux invariants, tous deux structurels désormais :

- la zone de travail est sur le même système de fichiers que la destination ;
- rien d'incomplet ne porte une extension que le scan reconnaît.

Un scan qui croiserait un fichier à moitié écrit l'indexerait comme une piste
tronquée, avec une empreinte qui cesserait d'être vraie une seconde plus tard.
Ces deux invariants rendent ce croisement impossible plutôt qu'improbable.

## Décision 7 — la piste existe à la validation

La validation ne se contente pas de poser le fichier : elle l'applique par le
chemin ordinaire du scan, et la piste existe immédiatement.

Le serveur sait déjà faire exactement cela depuis la correction des métadonnées
— lire un fichier et l'appliquer sans passer par un scan complet. Réutiliser ce
chemin plutôt qu'en écrire un second donne trois choses d'un coup : la piste est
consultable dès la réponse, le flux d'événements de bibliothèque l'annonce comme
n'importe quel autre changement, et ce que la validation écrit est par
construction ce qu'un scan aurait écrit.

L'alternative — poser le fichier et attendre le prochain scan — laisserait le
client devant un téléversement réussi et un catalogue qui l'ignore, sans rien
pour savoir combien de temps.

### L'extension n'est pas une preuve

N'importe quel fichier renommé en `.flac` franchit le contrôle de la négociation,
qui ne voit qu'une chaîne. La validation, elle, lit réellement le fichier : si
l'extraction échoue, le téléversement est refusé et la zone de travail nettoyée.

Ce contrôle n'est pas une pièce à écrire — c'est la décision ci-dessus.
Appliquer le fichier par le chemin du scan, c'est le faire lire par le même
extracteur ; ce qu'il ne sait pas lire n'entre pas.

### L'ordre, parce qu'il n'y a pas de transaction commune

SQLite et le système de fichiers ne partagent aucune transaction. L'ordre est
donc une décision, pas un détail d'implémentation :

1. la taille reçue correspond à la taille annoncée ;
2. l'empreinte recalculée correspond à l'empreinte annoncée ;
3. renommage atomique vers son nom définitif ;
4. le fichier s'ouvre et s'extrait réellement ;
5. une seule transaction SQLite : la piste et l'événement de bibliothèque.

Une panne entre 3 et 5 laisse un fichier orphelin sur le disque, que le prochain
scan ramasse — c'est très exactement ce qu'un scan sait faire, et le fichier
porte déjà son nom définitif. L'ordre inverse laisserait une ligne de catalogue
qui pointe vers rien : un état que rien ne répare tout seul et que chaque lecture
rencontre.

L'extraction vient après le renommage et non avant, contrairement à ce que la
première version de cette RFC posait. L'extracteur choisit sa branche DSD sur
l'extension du fichier : lire un fragment nommé `.part` prendrait la mauvaise
branche et refuserait un DSF parfaitement valide. Et la fenêtre que cet ordre
ouvre est vide — un fichier que l'extracteur ne sait pas lire n'est pas
indexable par un scan non plus, puisque c'est le même extracteur. Ce qui ne
s'ouvre pas est retiré, sans que rien ait pu s'y accrocher entre-temps.

L'événement de bibliothèque appartient à la même transaction que la piste, ce que
`record_library_event` impose déjà par sa signature — elle prend la transaction,
pas la connexion.

### Un rejeu ne duplique pas la piste

Une validation rejouée après que le renommage a réussi — la transaction a échoué,
ou un scan est passé entre-temps — retrouve le fichier à son nom définitif.
L'identité d'une piste se résout d'abord par son chemin, et le chemin est
maintenant celui d'une piste qui existe : l'application la met à jour au lieu
d'en créer une seconde. C'est le cas ordinaire d'un scan, pas un cas
particulier du téléversement.

Ce qu'un rejeu peut produire en double, c'est l'événement. Il n'a pas de clé
d'idempotence, et il n'en reçoit pas ici : un `upsert` énonce un état, pas un
delta, donc un client qui le reçoit deux fois relit deux fois la même piste et
n'en tire rien de faux. Importer l'`operation_id` du RFC-003 coûterait la
colonne, sa contrainte et la fusion des deux vocabulaires que le RFC-007 sépare
délibérément, pour supprimer une relecture. Le jour où un genre d'événement
énoncera un delta plutôt qu'un état, la question se reposera — et ce sera une
question du RFC-007.

### Un fichier reçu n'a pas de scan, et le dit

Le chemin d'application demandait l'identifiant du scan qui écrit la piste. Un
téléversement n'en a pas : aucun parcours n'a eu lieu.

Trois issues, et deux sont mauvaises. Inscrire un faux job de scan mettrait dans
l'historique un fait qui n'a pas eu lieu, et ce mensonge ressortirait ailleurs —
métriques, purge, écran d'administration. Élargir la contrainte `CHECK` de
`scan_job.trigger` obligerait à reconstruire la table, pour déclarer qu'un
téléversement est une sorte de scan alors qu'il n'en est pas une.

La colonne est déjà `NULL`able. C'est la signature qui était trop étroite : elle
prend maintenant un identifiant optionnel, et `NULL` veut dire **cette ligne ne
vient pas d'un scan** — jamais « scan inconnu ». Appliquer au catalogue n'est pas
intrinsèquement une opération du scanner, et la signature est l'endroit où cela
cesse d'être sous-entendu.

Ce sens n'existe que si la balayeuse de fin de scan le respecte, et elle ne le
faisait pas : elle traitait `NULL` comme « jamais vu », donc comme disparu. Un
fichier reçu pendant qu'un scan tourne aurait été marqué indisponible par ce
scan — dont le parcours avait commencé avant que le fichier existe — et une
suppression aurait été annoncée à tous les clients quelques secondes après son
arrivée. Une ligne sans scan n'est donc balayée que si elle précède le scan
courant, cas où le parcours aurait effectivement dû la trouver. Le premier scan
qui passe sur un fichier reçu l'estampille, et il rejoint le cas ordinaire pour
de bon.

### La réponse rend l'identifiant et l'empreinte

La réponse de validation porte l'identifiant de la piste créée et son
`full_hash`.

C'est ce que le desktop a demandé, et l'argument est juste : à cet instant, et à
cet instant seulement, les deux côtés savent que leur fichier et cette piste sont
les mêmes octets. Le lien de réconciliation s'écrit gratuitement. Sans ces deux
champs, le client attend le prochain parcours de catalogue pour découvrir sa
propre piste, puis relit le fichier entier pour l'y relier — pour une information
que le serveur détenait déjà au moment de répondre.

### Ce dont la réussite ne dépend pas

La réponse ne dépend que de ce que le serveur a lu dans le fichier lui-même.
Toute opération ultérieure sur cette piste, d'où qu'elle vienne, est un
changement de bibliothèque comme un autre et voyage par le flux du RFC-007 —
elle ne retient jamais la réponse.

Le mot « enrichissement » circule dans les schémas d'architecture ; il ne
correspond à rien dans le code, et le RFC-007 dit pourquoi il n'entre pas dans le
vocabulaire tant que personne ne l'a décidé. Cette RFC ne l'introduit pas
davantage : elle pose l'invariant — l'indexation locale est synchrone, tout le
reste est un événement — qui reste vrai si une telle étape existe un jour.

## Décision 8 — ce que la réception ne fait pas

**`uploadRole` reste faux.** La façade Subsonic n'accepte pas de téléversement,
et l'annoncer parce qu'une route native existe ferait essayer des clients qui
échoueraient. Le champ décrit la façade, pas le serveur.

**Pas de transcodage à l'entrée.** Le serveur stocke ce qu'on lui donne. Un
plafond de qualité au téléversement est une décision du client, qui seul sait ce
que coûte sa liaison.

**Pas de remplacement.** Un téléversement dont l'empreinte est déjà connue
s'arrête à la décision 2. Substituer un fichier à un autre est une opération
différente, avec ses propres questions, et elle n'est pas demandée.

**La suppression est hors périmètre.** La première version disait « pas de
suppression », ce qui figeait plus que cette RFC n'a le droit de figer. Un client
capable de remplir un disque et incapable de retirer ce qu'il vient d'y déposer
par erreur sera rapporté comme un défaut, pas comme une limite assumée — et le
nettoyage d'une session expirée ou refusée est déjà une suppression que le
serveur doit écrire de toute façon. Ce que cette RFC décide, c'est qu'elle
n'ouvre pas de route de suppression. Pas qu'il n'y en aura jamais.

## Ce que cette RFC change ailleurs

**`library_event` doit porter l'appareil d'origine.**

La table n'a ni `operation_id` ni appareil d'origine, et son commentaire de
migration dit pourquoi : « nothing here is client-originated yet — a scan writes
every row ». Ce n'était déjà plus vrai à la fusion de l'écriture de métadonnées,
et le téléversement en éloigne encore le serveur.

Sans cette colonne, un client reçoit **son propre téléversement** par le flux
comme une piste qu'il découvre, et le traite comme une découverte. Le journal
utilisateur porte `origin_device_id` depuis son premier jour, exactement pour
cela. La colonne manquante est une dette du RFC-007 plutôt qu'une question de
celui-ci ; elle y est notée.

## Un coût côté client, à assumer plutôt qu'à découvrir

`full_hash` couvre le fichier entier. Le condensé que le desktop tient localement
est un condensé tête-et-queue — le même compromis que `quick_hash` ici, et pour
la même raison : ne pas relire la bibliothèque entière à chaque scan.

« Téléverser tout ce qui manque » impose donc une lecture intégrale de la
bibliothèque locale, une fois. C'est acceptable, et c'est déjà ce que demande la
réconciliation, mais c'est une décision à prendre les yeux ouverts plutôt qu'une
surprise à l'implémentation.

## Ce qui reste ouvert

- Les valeurs : plafond par fichier, quota par bibliothèque, taille de fragment,
  taille d'un lot de négociation, sessions simultanées par compte, expiration
  d'une session.
- Le nettoyage des zones de travail abandonnées : à l'expiration, au démarrage,
  ou les deux.
- Le partage d'un même blob entre bibliothèques, qui demande d'abord de décider
  ce que ces frontières signifient.
