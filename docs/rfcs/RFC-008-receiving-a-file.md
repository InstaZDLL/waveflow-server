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

## Décision 2 — l'empreinte avant les octets

Le téléversement se négocie avant de transférer quoi que ce soit. Le client
annonce ce qu'il a — `full_hash`, taille, extension — et le serveur répond
`known` ou `unknown`.

`known` clôt l'échange : le serveur détient déjà ces octets et rien n'est
transféré. C'est le mécanisme que le desktop a proposé et il a raison — sans
lui, une bibliothèque largement partagée entre les deux côtés s'envoie des
gigaoctets pour découvrir qu'ils étaient déjà là.

C'est aussi le seul moment où un refus est bon marché. **Format non supporté,
taille au-dessus du plafond, quota dépassé, bibliothèque fermée : tout se dit
ici**, avant le premier octet, plutôt qu'après le dernier.

Les formats acceptés sont ceux que le scanner sait indexer —
`waveflow_core::scanner::AUDIO_EXTENSIONS` et rien d'autre. Accepter un fichier
que le catalogue ne saura pas lire serait accepter de l'occuper de l'espace pour
rien.

## Décision 3 — l'empreinte annoncée n'établit jamais rien

Le serveur recalcule le `full_hash` à la validation et refuse si les deux
diffèrent, en jetant ce qui a été reçu.

L'empreinte annoncée sert à **éviter un transfert**, jamais à **établir une
identité**. La distinction n'est pas théorique : une identité fondée sur ce que
le client affirme laisserait n'importe quel membre autorisé faire passer un
fichier pour un autre, et la déduplication de la décision 2 deviendrait un moyen
de substitution plutôt qu'une économie.

## Décision 4 — fragmenté, et la limite globale ne bouge pas

Le routeur pose `DefaultBodyLimit::max(16 * 1024)`. Ce n'est pas un oubli : une
API dont chaque route accepte seize kilo-octets ne peut pas être noyée par un
corps de requête, et cette propriété vaut d'être gardée.

La route de téléversement porte donc **sa propre borne, par fragment**, et ne
relève pas la limite du routeur. Un plafond global de plusieurs mégaoctets
donnerait à chaque route du serveur — y compris celles qui n'attendent qu'un
identifiant — une surface qu'aucune n'a demandée.

Fragmenté aussi parce que le desktop l'a bien posé : plusieurs milliers de
morceaux sur une liaison domestique ne passent pas en une fois. Une session
porte l'index du fragment attendu, de sorte qu'une coupure se reprend là où elle
s'est arrêtée plutôt qu'au début.

## Décision 5 — le serveur nomme le fichier

Le client ne propose jamais de chemin, pas même un nom.

Toute une famille de problèmes disparaît avec cette phrase : la traversée de
répertoire, l'écrasement d'un fichier que l'opérateur avait rangé lui-même, la
collision entre deux téléversements simultanés. Aucun de ces problèmes n'a besoin
d'être résolu s'il n'est jamais posé.

Le fichier atterrit sous un sous-répertoire que le serveur possède, **à
l'intérieur de la racine de la bibliothèque**, pour que le scan ordinaire le
trouve sans qu'on lui apprenne un second endroit où regarder. Jamais un lien
symbolique — le parcours les refuse déjà, délibérément.

## Décision 6 — rien de partiel n'est jamais visible

Les fragments s'écrivent dans une zone temporaire hors de la racine de la
bibliothèque. Le fichier n'entre à sa place définitive qu'après vérification de
son empreinte, par un déplacement sur le même système de fichiers.

Un scan qui croiserait un fichier à moitié écrit l'indexerait comme une piste
tronquée, avec une empreinte qui cesserait d'être vraie une seconde plus tard.
La zone temporaire est ce qui rend ce croisement impossible plutôt
qu'improbable.

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

## Décision 8 — ce que la réception ne fait pas

**`uploadRole` reste faux.** La façade Subsonic n'accepte pas de téléversement,
et l'annoncer parce qu'une route native existe ferait essayer des clients qui
échoueraient. Le champ décrit la façade, pas le serveur.

**Pas de transcodage à l'entrée.** Le serveur stocke ce qu'on lui donne. Un
plafond de qualité au téléversement est une décision du client, qui seul sait ce
que coûte sa liaison.

**Pas de suppression.** Recevoir un fichier n'est pas gérer une bibliothèque.
Retirer ce qui a été déposé passe par l'opérateur et son système de fichiers,
comme le reste.

**Pas de remplacement.** Un téléversement dont l'empreinte est déjà connue
s'arrête à la décision 2. Substituer un fichier à un autre est une opération
différente, avec ses propres questions, et elle n'est pas demandée.

## Ce qui reste ouvert

- Les valeurs : plafond par fichier, quota par bibliothèque, taille de fragment.
- L'expiration d'une session inachevée, et le nettoyage de sa zone temporaire.
- La convention de rangement : par tags — artiste, album, titre — ou par
  empreinte. Les tags donnent une arborescence qu'un opérateur reconnaît et que
  la pochette de dossier suit ; l'empreinte ne ment jamais mais ne se lit pas.
  À trancher à l'implémentation, avec un repli quand les tags manquent.
