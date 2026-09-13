# RFC-010 — Le scrobbling externe

- **Statut** : Proposed
- **Implémentée par** : rien encore. Quand du code existera, c'est cette ligne
  qui nommera les PR, et le champ *Statut* ci-dessus ne basculera pas — il ne
  bascule jamais dans ce projet.
- **Date** : 2026-09-13
- **Révisée** : 2026-09-13, après revue externe. Les décisions 2 à 6 ont changé,
  et chacune dit ce que la version antérieure affirmait de faux plutôt que de
  l'effacer.
- **Auteurs** : projet WaveFlow
- **Dépend de** : [RFC-002](RFC-002-waveflow-server-v2.md),
  [RFC-003](RFC-003-waveflow-sync-v2.md)
- **Clôt** : le dernier des cinq manques face à Navidrome qui se remarque à
  l'usage quotidien, et le seul point du plan web qu'aucun lot ne portait

## Problème

Une écoute ne quitte pas le serveur. `scrobble_with_context`
(`src/services/playback.rs`) écrit une ligne dans `play_event` et entretient
`now_playing` ; `GET /api/v2/history` les relit. C'est tout. Last.fm,
ListenBrainz et Maloja ne reçoivent rien.

C'est le seul des cinq écarts face à Navidrome dont l'absence se voit tous les
jours : un auditeur qui tient un profil d'écoute depuis dix ans ne l'abandonne
pas parce qu'il a changé de serveur.

Ce n'est pourtant pas une fonction qu'on branche. **Le serveur ne passe
aujourd'hui aucun appel sortant** — `Cargo.toml` ne porte aucun client HTTP — et
lui en faire passer change sa posture : une installation personnelle derrière un
proxy devient un logiciel qui parle à des tiers, avec des identifiants, une file
qui survit aux redémarrages et des échecs qui ne sont pas les siens. Chacune de
ces phrases est une décision, et c'est pour cela que cette RFC précède le code.

## Décision 1 — un seul point d'accroche, celui où les deux surfaces convergent

`scrobble_with_context` et rien d'autre. L'API native et la façade Subsonic y
arrivent déjà toutes les deux : c'est ce que M4 a construit, et brancher le
transfert ailleurs le dupliquerait aussitôt.

**La mise en file est écrite dans la transaction qui écrit `play_event`**, sous
le même verrou d'écriture. Une écoute ne peut donc pas être transmise sans avoir
été enregistrée, ni enregistrée sans être mise en file. Envoyer depuis le
gestionnaire HTTP, après la transaction, laisserait exactement la fenêtre que
tout le reste de ce serveur s'applique à fermer.

L'idempotence vient avec : `claim_operation` rejoue déjà une opération répétée
en annulant la transaction. Une écoute rejouée n'écrit pas de seconde ligne,
donc elle ne produit pas non plus de seconde mise en file.

## Décision 2 — une file durable, qui fige ce qui a été écouté

Une table `scrobble_outbox`, drainée par une tâche de fond de la forme des six
autres — `spawn_upload_sweeper`, `spawn_canvas_sweeper`, `spawn_artwork_sweeper`
et leurs voisines, toutes démarrées dans `src/main.rs`.

L'alternative — appeler le service distant pendant la requête — est écartée pour
trois raisons qui tiennent chacune seule : la requête d'un client attendrait un
tiers dont le serveur ne contrôle ni la latence ni la disponibilité ; un
redémarrage perdrait ce qui n'était pas parti ; et un service muet pendant une
journée ferait échouer des écoutes que le serveur a pourtant enregistrées
correctement.

**Chaque ligne porte une enveloppe immuable** : `played_at`, le titre, les
artistes crédités, l'album et son artiste quand ils existent, la durée, et
l'identifiant MusicBrainz d'enregistrement s'il est connu. Le `play_event_id`
reste, comme provenance. Le lien vers la piste ne sert plus à lire les
métadonnées au moment du drainage.

**Une version antérieure de cette décision disait le contraire** : « pas de
copie de la piste, la piste se relit, et deux modèles d'une même chose finissent
par se contredire ». Cet argument vaut contre une *projection* d'une ligne
vivante ; il est faux ici, parce que l'instantané ne doit justement pas suivre la
piste. Depuis [#186](https://github.com/InstaZDLL/waveflow-server/pull/186) un
membre corrige titre et artistes, et une correction de liste réécrit
`track_participant` : relire au drainage enverrait ce que la piste est devenue,
pas ce qui a été entendu. Une enveloppe figée dit la vérité d'un moment, ce qui
est exactement ce qu'un historique d'écoute enregistre.

Cela répond aussi à une question que la version antérieure laissait ouverte : une
piste supprimée entre l'écoute et l'envoi ne vide plus la ligne de file.

## Décision 3 — on met en file les écoutes, et « en écoute » ne touche jamais la requête

`play_event.submission` distingue déjà les deux, et la distinction est la même
chez les destinataires : `updateNowPlaying` chez Last.fm, `playing_now` chez
ListenBrainz, où il est explicitement temporaire et n'enregistre rien.

**Une soumission est mise en file. Un « en écoute » n'est pas mis en file.** Il
n'a de valeur que pendant qu'il est vrai : le retransmettre dix minutes plus tard
annoncerait une piste que l'auditeur a quittée depuis longtemps. Garantir la
livraison d'une information périmée est pire que de la perdre.

**Mais « sans file » ne veut pas dire « dans la requête ».** Aucun appel HTTP
vers un tiers ne part du chemin d'une requête client, jamais, quelle qu'en soit
la raison. Après le commit, l'« en écoute » est déposé dans un canal mémoire
borné que le drainage consomme. Canal plein, ou redémarrage : l'événement est
perdu, et c'est le comportement voulu.

## Décision 4 — deux étages d'identifiants, et le binaire n'en porte aucun

Le précédent existe et il est bon : le mot de passe Subsonic dédié est chiffré
par `SecretBox` en ChaCha20-Poly1305 sous la clé d'instance.

Mais un seul étage ne suffit pas, et la version antérieure de cette décision le
supposait. ListenBrainz et Maloja se contentent d'un jeton par personne. Last.fm
demande trois choses : une clé d'API et un secret partagé qui identifient
*l'application*, puis une clé de session qui identifie *le compte*. Donc :

- **Au niveau du déploiement** : la clé d'API et le secret du fournisseur, posés
  par l'opérateur. **Jamais embarqués dans le binaire** — ce dépôt est publié
  sous AGPL, et un secret dans un binaire public n'est pas un secret. Un
  opérateur qui veut Last.fm déclare ses propres identifiants d'application.
- **Au niveau du compte** : la clé de session, ou le jeton personnel selon le
  destinataire, scellé sous la clé d'instance.

**Par compte, pas par bibliothèque.** Une écoute appartient à une personne ; un
réglage par bibliothèque poserait une question à laquelle personne ne tient —
sous quel profil compte une écoute dans une bibliothèque partagée.

**Jamais relus en clair par l'API.** Comme pour les jetons d'API, on les
remplace, on ne les relit pas. La sauvegarde reste ce que `CLAUDE.md` dit déjà :
`data/waveflow.db` et `data/instance.key` vont ensemble, sans quoi ces secrets ne
sont plus déchiffrables — et c'est le comportement voulu.

### Délier et relier : une génération, pas une paire

`scrobble_link` porte un identifiant propre, et **`scrobble_outbox` référence ce
lien-là**, pas le couple `(compte, destination)`.

Sans cela : vingt écoutes en attente, je délie mon compte Last.fm, j'en relie un
autre — et les vingt partent sur le profil du second. Le compte et la destination
sont les mêmes ; l'autorisation, non.

Délier désactive le lien et fait finir ses lignes en attente dans un état
terminal. Relier crée une génération nouvelle, qui ne voit rien de l'ancienne.
Une requête déjà émise, elle, ne se rappelle pas : c'est la limite du procédé et
elle est nommée ici plutôt que découverte.

## Décision 5 — la frontière : exactement une fois chez nous, au plus une tentative ambiguë chez eux

**C'est la décision que la version antérieure ratait, et elle se contredisait.**
Elle disait à la fois « un doublon est pire qu'une perte » et « réseau ou 5xx :
on retente ». Les deux sont incompatibles pour un POST sans clé d'idempotence, et
ni `track.scrobble` chez Last.fm ni `submit-listens` chez ListenBrainz n'en
offrent une.

Le cas qui tranche : le serveur émet la requête, le destinataire l'enregistre,
la connexion tombe avant que la réponse revienne. Une panne réseau et une
réussite dont l'accusé s'est perdu sont **indiscernables**. Retenter produit un
second scrobble.

**La file transactionnelle donne l'atomicité entre WaveFlow et sa propre file.
Elle ne peut pas donner l'atomicité entre WaveFlow et un tiers.** Cette frontière
est réelle, aucun réglage ne la déplace, et la RFC la reconnaît au lieu de
promettre au-delà.

Ce qui est donc garanti, et rien de plus :

- **Exactement une mise en file**, côté WaveFlow. Une ligne est unique par
  `(play_event, lien)`, contrainte tenue par le schéma et pas seulement par le
  code.
- **Au plus une tentative pour un résultat ambigu**, côté destinataire. On ne
  retente que lorsqu'on **sait** que la requête n'est pas passée.

La philosophie ne change pas — **préférer une perte à un doublon** — parce qu'une
écoute perdue manque à un compteur, tandis qu'une écoute envoyée deux fois abîme
un historique public que la personne tient parfois depuis des années et ne peut
pas corriger facilement.

**L'ordre n'est pas garanti et n'a pas à l'être** : chaque soumission porte son
propre `played_at`, que les trois destinataires acceptent. Sérialiser la file par
compte coûterait un blocage de tête de file pour une propriété que personne
n'observe.

## Décision 6 — le verdict vient de l'adaptateur, jamais du code HTTP

La version antérieure classait par statut HTTP. C'est faux pour au moins deux des
trois destinataires : **Last.fm répond `200` en portant un code applicatif** dans
le corps — session invalide, indisponibilité temporaire, limite de débit — et
Maloja signale ses refus dans son JSON. Lire le statut seul prendrait un échec
pour une réussite.

Chaque adaptateur de fournisseur rend donc **un verdict**, et le drainage ne
connaît que ces cinq mots :

| Verdict | Conduite |
| --- | --- |
| `Accepted` | la ligne part, c'est fini |
| `Retryable` | recul croissant avec gigue, nombre de tentatives borné, `Retry-After` honoré quand il est là |
| `AuthBroken` | le lien est marqué rompu, l'API le dit au compte, et **ses lignes en attente finissent en état terminal** |
| `PermanentReject` | la ligne part, avec un journal : aucune reprise ne corrigera une charge que le destinataire refuse |
| `Ambiguous` | **aucune reprise automatique** — état terminal `uncertain`, compté et lisible, par la décision 5 |

Le drainage ne sait donc rien de Last.fm, de ListenBrainz ni de Maloja. C'est ce
qui permet d'en ajouter un quatrième sans toucher à la file.

**Une file qui ne se vide pas est un défaut**, pas un état. Après les tentatives
bornées, la ligne est abandonnée et comptée, et ce compte est lisible.

## Décision 7 — ce que le serveur n'envoie pas

- **Rien de rétroactif à l'activation.** Brancher un compte n'envoie pas son
  historique : personne ne veut voir dix ans d'écoutes remonter d'un coup sur son
  profil, et un destinataire lit cela comme un abus.
- **Aucune piste qu'il ne sait pas nommer.** Sans titre ni artiste, la soumission
  est inutilisable et fausse les statistiques du destinataire.
- **Rien depuis un partage.** `/share/{token}` sert un visiteur sans compte ; il
  n'y a pas de profil à créditer.

## Décision 8 — la façade Subsonic ne bouge pas

Le contrat est gelé pour `v2.0-beta`. Aucun champ nouveau, aucune méthode
nouvelle : un client Subsonic continue d'appeler `scrobble`, et ce qui se passe
ensuite ne le regarde pas. C'est déjà le cas pour tout ce que cette façade
ignore.

## Décision 9 — la configuration est une route, pas une variable d'environnement

`src/config.rs` porte ce qui appartient au déploiement. Un lien de scrobbling
appartient à un compte, donc il se pose par l'API native — et par la CLI pour un
opérateur qui prépare un serveur sans navigateur, comme pour le mot de passe
Subsonic.

Ce qui reste au déploiement : les identifiants d'application de la décision 4, le
délai d'attente sortant, le plafond de tentatives, l'intervalle de drainage, et
la base d'URL des destinations auto-hébergées.

## Décision 10 — la surface sortante est bornée, et c'est la décision qui compte

C'est le vrai risque de cette RFC : un serveur qui appelle une URL est un serveur
qu'on peut faire appeler une URL. Maloja et ListenBrainz s'auto-hébergent, donc
il existera un champ d'URL, et il ne peut pas être libre.

- **HTTPS seulement**, sauf pour une cible explicitement déclarée par l'opérateur
  en clair sur son propre réseau.
- **L'URL de base est le réglage de l'opérateur**, jamais celui d'un compte. Un
  membre choisit sa destination parmi celles que le serveur connaît, et ne la
  décrit pas.
- **Aucune redirection suivie** vers un autre hôte que celui demandé.
- **Délais et taille de réponse bornés**, comme toute autre entrée-sortie ici.
- **Rien du corps de la réponse n'est renvoyé au client** : il va au journal, et
  ce que l'API montre est un état, pas un écho.

## Ce que cette RFC change ailleurs

- **Une dépendance sortante entre dans `Cargo.toml`** pour la première fois.
  C'est le point le plus lourd de conséquences, et il mérite d'être dit plutôt
  que découvert dans un `cargo tree`.
- **Deux tables** : `scrobble_link` et `scrobble_outbox`, en migrations datées,
  comme toujours.
- **Une septième tâche de fond**, démarrée dans `src/main.rs` avec les six
  autres.
- **La documentation** : une section du guide d'API, et une ligne dans
  `docs/web-client-gap-analysis.md`, dont le point 15 attend celle-ci.

## Ce qui reste ouvert

- **Quel destinataire d'abord.** ListenBrainz est le plus simple — un jeton dans
  l'en-tête d'autorisation, une soumission JSON — et Last.fm demande la signature
  et la session de la décision 4. Le premier n'engage pas le second, et c'est par
  lui qu'il faut commencer.
- **Si la profondeur de la file se montre** dans l'API, ou seulement son état :
  sain, rompu, ou avec des envois incertains.
- **Ce qu'un état `uncertain` permet de faire ensuite.** Le compte le voit ; rien
  ne dit encore s'il peut demander une seconde tentative en connaissance de
  cause, ce qui serait le seul doublon que cette RFC accepterait — parce qu'il
  serait choisi.
