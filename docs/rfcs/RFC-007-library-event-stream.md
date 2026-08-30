# RFC-007 — Flux d'événements de bibliothèque

- **Statut** : Proposed
- **Implémentée par** : [#145](https://github.com/InstaZDLL/waveflow-server/pull/145)
  (le flux et son filigrane), [#152](https://github.com/InstaZDLL/waveflow-server/pull/152)
  (l'appareil d'origine). Le champ *Statut* ci-dessus ne bascule jamais dans ce
  projet : c'est cette ligne qui dit ce qui tourne, et elle se vérifie — une PR
  se lit, un mot de statut ne s'audite pas.
- **Date** : 2026-08-25
- **Auteurs** : projet WaveFlow
- **Dépend de** : [RFC-002](RFC-002-waveflow-server-v2.md),
  [RFC-003](RFC-003-waveflow-sync-v2.md)
- **Préalable de** : l'écriture des métadonnées d'une piste, la réception d'un
  fichier (RFC-008), et l'asset vidéo par piste

## Problème

WaveFlow Server tient deux états, et un seul sait se raconter.

L'**état utilisateur** — favoris, notes, écoutes, file, playlists, partages,
signets — converge par le journal de synchronisation décrit au RFC-003. Sept
types d'entité, contraints par `CHECK` sur `sync_event` parce que le
vocabulaire est un contrat et non du texte libre. Un client suit son curseur et
sait ce qu'il a manqué.

L'**état bibliothèque** — pistes, albums, artistes, pochettes — n'a aucun flux.
Le constat est vérifiable plutôt qu'argumenté : `scanner.rs` et `catalog.rs` ne
contiennent **aucun appel au journal**. Le seul flux voisin,
`/api/v2/scans/{id}/events`, transporte la *progression* d'un job de scan — un
instantané, des ticks, puis il meurt avec le job. Il dit qu'un scan avance ; il
ne dit jamais qu'une piste a changé.

Conséquence pour un client : la seule manière d'apprendre qu'un catalogue a
bougé est de le resonder. Un client qui compare des compteurs attrape une piste
ajoutée ou retirée et rate tout le reste — un titre retagué, une année corrigée,
une pochette remplacée. C'est un contournement, et il est aveugle par
construction.

Trois demandes en attente butent toutes sur ce manque : écrire les métadonnées
d'une piste, recevoir un fichier, et attacher un asset vidéo. Aucune n'a
d'endroit où annoncer ce qu'elle a fait.

## Décision 1 — deux flux, jamais un

L'état bibliothèque reçoit sa propre table et sa propre séquence. Il ne rejoint
pas `sync_event`.

La raison est mécanique et non esthétique : `sync_event.cursor` est un
`INTEGER PRIMARY KEY AUTOINCREMENT`, c'est-à-dire **une séquence unique et
globale**, filtrée par utilisateur à la lecture. Un flux de bibliothèque qui la
partagerait ferait avancer, à chaque rescan, la séquence même contre laquelle
tous les curseurs de favoris se mesurent. Un scan de cinquante mille pistes
déplacerait le curseur de synchronisation de chaque compte du serveur.

La conséquence côté client est assumée : **deux curseurs**, un par compte pour
l'état utilisateur, un par bibliothèque pour l'état bibliothèque. C'est le but.
Ils n'avancent ni au même rythme ni pour les mêmes raisons, et les fusionner
empêcherait d'avancer l'un sans l'autre.

### L'asymétrie est la conception

Les deux flux ne sont pas deux moitiés symétriques :

| | État utilisateur | État bibliothèque |
| --- | --- | --- |
| Volume | petit | volumineux |
| Portée | un compte | une bibliothèque, partagée |
| Origine | le client écrit | le serveur écrit, presque toujours |
| Machinerie | identifiants d'opération, rejeu, conflit | rien de tout ça |

Presque toujours : un `PATCH` de métadonnées et un téléversement sont des
changements de bibliothèque **venus du client**, et leur file hors-ligne les
rejouera. Le flux a donc besoin d'idempotence pour une minorité de son trafic et
d'aucune pour la majorité. C'est exactement ce que `MutationContext::server_generated()`
modélise déjà côté utilisateur, aujourd'hui : un contexte de mutation sans
origine cliente. Le mécanisme se transpose sans être réinventé.

## Décision 2 — l'appartenance est évaluée à la lecture

La lecture du flux joint `library_member` **au moment de la requête**, comme
chaque projection du serveur.

Cela règle la révocation sans rien construire. Un membre qui perd l'accès cesse
de recevoir les événements de cette bibliothèque à la requête suivante, et
rétroactivement : il n'y a pas de liste de révocation à tenir, parce qu'il n'y a
jamais eu de liste d'abonnés. L'appartenance courante *est* le filtre.

Ce qu'un client a déjà téléchargé reste sur son disque, et rien ne peut le
reprendre. Mais il ne peut plus rien en résoudre : le 404 confond une ressource
absente et une ressource étrangère, ce que le RFC-002 pose comme règle. La fuite
est bornée à ce que le client avait déjà, ce qui est la propriété de n'importe
quel catalogue mis en cache.

## Décision 3 — un flux par bibliothèque, des événements typés

Un seul flux par bibliothèque, dont les événements portent un genre. Pas deux
flux thématiques.

La tentation est de séparer ce qui vient du scan de ce qui viendrait d'un
enrichissement, au motif qu'on veut l'un tout de suite et l'autre à loisir. Mais
deux flux, c'est deux curseurs *par bibliothèque* — soit exactement le couplage
que la décision 1 cherche à éviter, réintroduit un cran plus bas. Un genre typé
suffit : un curseur est une position, pas une file d'attente, donc ignorer un
genre ne coûte rien et ne retient rien.

Le vocabulaire est contraint par `CHECK`, comme celui du journal utilisateur, et
pour la même raison : il fait partie du contrat.

- `entity_type` : `track`, `album`, `artist`
- `action` : `upsert`, `delete`

### L'appareil d'origine, tranché

Un événement porte l'appareil qui l'a demandé, quand un client l'a demandé.

La table a d'abord été créée sans, au motif écrit dans sa migration qu'aucune
ligne ne venait d'un client. L'écriture de métadonnées a rendu ce motif faux et
la réception d'un fichier ([RFC-008](RFC-008-receiving-a-file.md)) l'a éloigné
davantage : sans cette colonne, un client relit son propre téléversement sur le
flux comme une piste qu'il vient de découvrir, et la traite comme telle. Le
journal utilisateur porte `origin_device_id` depuis son premier jour, exactement
pour cela.

`NULL` garde son sens simple : personne ne l'a demandé. Un scan écrit `NULL`, et
un client qui ne nomme pas d'appareil aussi — l'en-tête est facultatif, et celui
qui préfère recevoir ses propres changements se contente de l'omettre.

L'appartenance de l'appareil est vérifiée avant d'être écrite. Sans ce contrôle,
un compte pourrait attribuer ses écritures à l'appareil d'un autre — et tout
client qui filtre ses propres changements hors du flux écarterait alors ceux de
quelqu'un d'autre.

**La charge utile d'un `upsert` de piste porte son `full_hash`.** C'est le point
que rien d'autre ne fournit, et il mérite d'être expliqué.

### Une édition externe déplace l'empreinte, en silence

L'écriture de métadonnées décrite par ailleurs ne réécrit aucun fichier : la
correction vit à côté de la piste, donc le `full_hash` ne bouge pas et un lien de
réconciliation établi par un client reste valide. Ce n'est pas là que le
problème se pose.

Il se pose quand l'opérateur retague lui-même, avec un outil externe. La
condition de saut du scanner exige `existing.full_hash == input.full_hash` ; des
octets différents la font échouer, l'application tourne, et **la piste garde son
identifiant pendant que son empreinte change**. Aucun client n'en est informé
aujourd'hui.

Porter le `full_hash` dans la charge utile suffit : le client détient l'ancien,
compare, et sait que son lien est à revérifier. Un genre d'événement dédié
serait plus de vocabulaire pour la même information.

## Décision 4 — la rétention est une question du premier jour

Le journal utilisateur n'a aucune compaction aujourd'hui, et l'assume : le
commentaire de `src/sync.rs` note qu'elle « rognera la tête du journal pour tout
le monde » lorsqu'elle arrivera. Ce report est tenable parce que les mutations
utilisateur sont rares.

Le flux de bibliothèque n'a pas ce luxe. Un premier scan de cinquante mille
pistes produit cinquante mille événements avant qu'aucun client n'ait lu quoi
que ce soit. La rétention n'y est donc pas une question pour plus tard.

Le précédent à reprendre existe déjà côté utilisateur : lorsqu'un curseur
précède le plus vieil événement conservé, le serveur ne bricole pas, il répond
que le curseur a expiré et que le client doit repartir d'un instantané. Pour
l'état bibliothèque, cet instantané n'est pas une route à écrire : c'est le
catalogue lui-même, que `/api/v2/libraries/{id}/tracks` et les routes de
parcours servent déjà.

Les valeurs exactes — âge, nombre d'événements conservés par bibliothèque — sont
laissées ouvertes ; c'est un réglage, pas une décision de forme.

## Décision 5 — ce que le flux ne transporte pas

**Pas d'enrichissement.** Le serveur ne va chercher aucune métadonnée
extérieure, par décision : il stocke et sert ce qu'on lui donne. Le mot apparaît
dans les schémas d'architecture qui circulent ; il ne correspond à rien dans le
code, et le laisser dans le vocabulaire promettrait une capacité que personne n'a
décidée. Si elle est décidée un jour, elle entrera comme un genre supplémentaire,
ce que la décision 3 rend possible sans rien casser.

**Pas d'événement pour une ressource adressée par contenu.** Une pochette et un
canvas sont immuables par hachage : les octets derrière une empreinte ne changent
jamais, donc il n'y a rien à notifier. Seul le *lien* d'une piste vers une
ressource est un changement, et il voyage dans l'`upsert` de la piste.

**Jamais d'état utilisateur.** Un favori, une note, une écoute restent dans le
journal du RFC-003. Le flux de bibliothèque ne les double pas, et un client qui
les recevrait deux fois les compterait deux fois.

## Ce que cette RFC change ailleurs

Elle **retire** de l'écriture de métadonnées sa ligne la plus risquée. Sans
flux propre, annoncer une piste modifiée aurait demandé d'admettre un type
`track` dans `sync_event` — donc de reconstruire cette table autour de sa
contrainte `CHECK`, SQLite ne sachant pas la modifier. C'est le mouvement avec
le `DELETE FROM` implicite qui a déjà coûté un correctif au projet. Une table
séparée n'y touche pas.

Elle **écarte** une décision prise trop vite : diffuser un événement par membre
de la bibliothèque dans le journal utilisateur. L'arithmétique la condamne — une
bibliothèque à quarante membres retaguée sur deux mille pistes écrirait quatre-
vingt mille lignes pour deux mille faits, dans une table sans compaction — et la
sémantique aussi : l'événement ne parle pas de l'utilisateur.

## Décision 6 — l'album a son événement, l'artiste n'en a pas besoin

Ouvert par la première version de cette RFC, tranché le 2026-08-30 sur un cas que
le desktop a nommé.

**L'album en émet un.** Le miroir incrémental du desktop saute un album dont le
`song_count` n'a pas bougé — c'est toute l'économie de son parcours. Un album
*simplement retagué* lui est donc invisible : une date de sortie apprise, une
pochette trouvée, un artiste d'album corrigé ne changent aucun compte. Sans
événement, la seule façon de l'apprendre est de redemander tous les albums, ce
qui annule exactement ce que le parcours économise.

Il n'est émis **que si la ligne a réellement été écrite**. L'`upsert` d'album
tourne une fois par piste appliquée, donc douze fois pour un album de douze
titres ; une clause `WHERE` sur le `DO UPDATE` compare les colonnes qui portent
du sens — `updated_at` exclu, que rien ne lit — et `RETURNING` ne rend une ligne
que si quelque chose a bougé. Un événement par album, jamais un par piste, et
rien du tout pour un rescan qui ne trouve aucun changement.

La charge est vide. Celle de la piste porte `full_hash` parce qu'aucune autre
surface ne l'expose ; tout ce qui décrit un album est sur l'album, que le client
relit en recevant l'événement.

**L'artiste n'en émet pas.** Les compteurs se dérivent, et une vérité stockée en
double périme. Le desktop réservait un cas — la photo d'artiste, qui ne se dérive
d'aucun `GROUP BY` — et ce cas n'existe pas : `artist.artwork_hash` est lu
partout dans ce serveur et **écrit nulle part**. Une photo d'artiste ne peut pas
changer parce qu'il n'y en a jamais. La question se rouvrira le jour où quelque
chose l'écrira, et ce jour-là ce sera cette ligne qu'il faudra relire.

## Ce qui reste ouvert

- Les valeurs de rétention.
- ~~Si des événements `album` et `artist` sont nécessaires ou seulement
  dérivables des événements `track` par le client.~~ **Tranché le 2026-08-30**,
  et différemment pour les deux — voir la décision 6.
- La forme exacte de l'acquittement. Le journal utilisateur acquitte par
  `(compte, appareil)` ; l'équivalent ici est `(appareil, bibliothèque)`, mais
  la table qui le porte reste à décider.
