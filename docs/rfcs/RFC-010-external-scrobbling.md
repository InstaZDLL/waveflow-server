# RFC-010 — Le scrobbling externe

- **Statut** : Proposed
- **Implémentée par** :
  [#191](https://github.com/InstaZDLL/waveflow-server/pull/191), la moitié
  durable — les deux tables, la mise en file dans la transaction qui écrit
  `play_event`, les cinq verdicts, et la tâche de drainage ; puis
  [#192](https://github.com/InstaZDLL/waveflow-server/pull/192), l'appel
  sortant — la surface bornée de la décision 10, l'adaptateur ListenBrainz, et
  le délai que la décision 6 promettait d'honorer sans que le verdict puisse le
  porter.
  Puis [#193](https://github.com/InstaZDLL/waveflow-server/pull/193), **les
  routes natives et la CLI** : un compte pose et retire son autorisation
  lui-même, lit l'état de sa file, et répond des écoutes dont personne ne sait
  si elles sont arrivées — la décision 13 demandait ce choix à une personne sans
  qu'aucune surface ne lui montre sur quoi. En ligne de commande, un opérateur
  pose et retire une autorisation et lit l'état d'une file — trois gestes, pour
  un serveur que nul n'a encore ouvert dans un navigateur ; répondre d'une
  écoute incertaine reste une décision que son auteur prend, et n'existe que
  sur la surface HTTP.
  Puis [#199](https://github.com/InstaZDLL/waveflow-server/pull/199), **le
  correctif de la décision 13** : le joker se compte sur `retried_at`, une
  colonne que l'entrée porte, et non sur la survie de la reprise qui l'a
  consommé — que la rétention emporte. Les quatre lecteurs qui lisaient la
  jointure lisent la colonne, et les bases déjà en service la reçoivent
  remplie.
  Puis [#200](https://github.com/InstaZDLL/waveflow-server/pull/200), **le
  reste** : la rétention de la file et sa huitième tâche de fond ; les
  destinations nommées, avec l'empreinte d'URL qui en fait l'identité, le
  rattrapage et la réconciliation au démarrage ; l'adaptateur Maloja, où
  l'enveloppe se verse sans perdre un crédit ; et le parcours d'autorisation
  Last.fm, deux routes et une table qui vit un quart d'heure.
  Enfin [#201](https://github.com/InstaZDLL/waveflow-server/pull/201), **la
  surface qui rend tout cela atteignable** : l'écran `/settings/scrobbling` du
  client web, où se posent et se retirent les autorisations par paire, s'ouvre
  le parcours Last.fm, et se répondent les écoutes que la décision 13 refuse de
  trancher. Ce chemin n'est pas un choix : `lastfm_callback` y redirige, et
  tant qu'il n'existait pas, un parcours mené à son terme atterrissait sur une
  page « introuvable » — le lien fait, et rien pour le dire. Deux corrections
  de serveur l'ont accompagné, toutes deux exigées par l'écran : une entrée
  `uncertain` nomme désormais son **instance** et non son seul destinataire —
  sans quoi la décision 13 pose sa question sans dire lequel des deux profils
  détient peut-être l'écoute — et un lien hérité que la réconciliation rompt
  reçoit un nom avant d'être rompu, au lieu de sortir de l'API avec une
  instance vide qu'aucun geste ne pouvait retirer.
  Le champ *Statut* ci-dessus ne bascule pas — il ne bascule jamais dans ce
  projet.
- **Date** : 2026-09-13
- **Révisée** : 2026-09-13, après revue externe. Les décisions 2 à 6 ont changé,
  et chacune dit ce que la version antérieure affirmait de faux plutôt que de
  l'effacer. Une seconde passe le même jour a tranché les trois questions qui
  restaient : décisions 11 à 13.
  Puis le 2026-09-14, après une seconde revue externe et une mesure : le joker
  d'une reprise se compte désormais sur l'entrée (décision 13), une destination
  peut exister en plusieurs instances nommées (décision 10), Last.fm obtient un
  protocole d'autorisation au lieu d'un secret collé (décision 11), et la
  rétention de la file est tranchée. Ces quatre-là sont implémentées le même
  jour, par #199 et #200 ci-dessus.
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
aujourd'hui aucun appel sortant**, et lui en faire passer change sa posture :
une installation personnelle derrière un proxy devient un logiciel qui parle à
des tiers, avec des identifiants, une file qui survit aux redémarrages et des
échecs qui ne sont pas les siens. Chacune de ces phrases est une décision, et
c'est pour cela que cette RFC précède le code.

> **Corrigé le 2026-09-13, en écrivant
> [#192](https://github.com/InstaZDLL/waveflow-server/pull/192).** Cette RFC
> ajoutait ici « — `Cargo.toml` ne porte aucun client HTTP — » et appelait plus
> bas l'arrivée d'une dépendance sortante « le point le plus lourd de
> conséquences ». `cargo tree -i reqwest` répond en trois lignes : **reqwest
> 0.12 arrive par `waveflow-core`, avec rustls, et est lié dans ce binaire
> depuis toujours.** `Cargo.toml` n'en *déclarait* aucun ; le binaire en *liait*
> un. Déclarer la dépendance n'a donc ajouté ni code ni seconde pile TLS —
> `Cargo.lock` n'a bougé que d'une ligne — et la marche était bien plus basse
> que ce paragraphe ne la dramatisait.
>
> Ce qui reste entièrement vrai, et qui était le vrai contenu de l'inquiétude :
> **le serveur ne parlait à personne**, et maintenant il le peut. La posture
> change ; c'est le graphe de dépendances qui ne changeait pas.
>
> Et le piège était réel, ailleurs que là où la RFC le cherchait : Cargo unifie
> les features, donc déclarer `reqwest` avec ses défauts aurait activé
> `default-tls` et fait entrer une seconde pile TLS **à côté** de rustls plutôt
> qu'à sa place. C'est la déclaration recopiée de `waveflow-core` qui l'évite.

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
délai d'attente sortant, le plafond de tentatives, l'intervalle de drainage, la
fenêtre de rétention, et les destinations elles-mêmes — leur nom autant que leur
URL, plusieurs par destinataire si l'opérateur le veut.

## Décision 10 — la surface sortante est bornée, et c'est la décision qui compte

C'est le vrai risque de cette RFC : un serveur qui appelle une URL est un serveur
qu'on peut faire appeler une URL. Maloja et ListenBrainz s'auto-hébergent, donc
il existera un champ d'URL, et il ne peut pas être libre.

- **HTTPS seulement**, sauf pour une cible explicitement déclarée par l'opérateur
  en clair sur son propre réseau.
- **L'URL de base est le réglage de l'opérateur**, jamais celui d'un compte. Un
  membre choisit sa destination parmi celles que le serveur connaît, et ne la
  décrit pas.
- **Aucune redirection suivie**, pas même vers l'hôte demandé. La première
  rédaction disait « vers un autre hôte », ce que le code ne fait pas : il pose
  `Policy::none()` et s'arrête à la première. C'est le texte qui promettait
  moins, et il avait tort de le promettre — un `302` vers le même hôte reste un
  chemin que personne n'a choisi, et l'échange qui convertit un jeton Last.fm en
  clé de session y enverrait une clé d'application et une signature.
- **Délais et taille de réponse bornés**, comme toute autre entrée-sortie ici.
- **Rien du corps de la réponse n'est renvoyé au client** : il va au journal, et
  ce que l'API montre est un état, pas un écho.

**Une exception, et c'est Last.fm qui l'impose.** La règle ci-dessus envoie au
journal ce que le fournisseur a répondu, parce qu'un corps d'erreur est ce qui
explique une panne. La réponse d'`auth.getSession` n'est pas un corps d'erreur :
elle contient la clé de session, qui vaut jusqu'à révocation. Elle ne va donc
pas au journal — ni entière, ni tronquée. Ce qui s'y écrit est un verdict et une
cause normalisée, comme pour toute autre soumission. La règle valait tant
qu'aucune réponse ne portait de credential ; ce parcours en introduit une, et la
règle se rétrécit d'autant plutôt que de la laisser passer.

### Plusieurs instances, nommées par l'opérateur

**Révisé le 2026-09-14.** « Un membre choisit sa destination parmi celles que le
serveur connaît » supposait un pluriel que le réglage ne donnait pas : un champ
d'URL par destinataire, donc une instance Maloja par serveur. ListenBrainz s'en
accommode — presque tout le monde vise l'instance publique — mais Maloja
s'auto-héberge par nature, et sur un serveur de famille chacun a la sienne. Un
champ unique force à partager une instance ou à renoncer.

L'opérateur déclare donc des destinations **nommées**, plusieurs par
destinataire s'il le veut, et un lien porte `(provider, destination)` au lieu du
seul `provider`. Le compte choisit un nom dans la liste que le serveur publie ;
il ne décrit toujours aucune URL, et la barrière de la décision 10 ne bouge pas
d'un pouce.

**La destination entre dans le chemin, elle aussi.** `PUT` et `DELETE`
s'adressaient à `/api/v2/scrobble-links/{provider}`, ce qui ne désigne plus rien
de précis dès qu'un destinataire a deux instances — et `DELETE` n'a pas de corps
où la nommer. Les deux deviennent
`/api/v2/scrobble-links/{provider}/{destination}`, et la liste publie le couple.
Aucun défaut implicite : un chemin sans destination est refusé plutôt que
rattaché à la première venue — le glissement que les deux paragraphes suivants
interdisent. La surface date d'hier et personne ne s'y est encore adossé, donc
ce contrat se corrige maintenant ou se traîne.

**Ce que ce couple déplace dans le code, dit ici pour qu'on ne le trouve pas
après.** Trois endroits tiennent aujourd'hui un lien pour identifié par son seul
destinataire, et aucun ne se contente d'une colonne de plus :

- `register_scrobble_target` range un adaptateur par `ScrobbleProvider`, donc
  deux instances Maloja n'en auraient qu'un, pointant sur une seule URL. Le
  registre se range par le couple, et le drain choisit par le couple.
- `unlink_scrobble_on` délie « le lien vivant de ce compte chez ce
  destinataire » — appelé par `link_scrobble` avant chaque nouvelle
  autorisation. Tel quel, lier `maloja/bob` délierait `maloja/alice` et
  annulerait sa file. La clause prend la destination, faute de quoi la
  fonctionnalité se détruirait elle-même au deuxième usage.
- L'index unique `scrobble_link(user_id, provider) WHERE status <> 'unlinked'`
  interdit tout simplement le cas : il devient `(user_id, provider,
  destination)`.

Ce n'est pas de l'implémentation laissée ouverte. C'est la forme que le couple
impose, et les trois se tiennent : en corriger deux sur trois donne un serveur
qui accepte deux destinations et en efface une.

**Un nom de destination s'écrit dans un chemin sans y être encodé.** Puisqu'il
devient un segment d'URL, il se borne à ce qui traverse un chemin sans
discussion — lettres ASCII, chiffres, `-`, `_` et `.`, au plus soixante-quatre
caractères — et `.` comme `..` sont refusés, ces deux-là étant des instructions
de navigation partout où un chemin se résout. Refuser au démarrage un nom qui
sort de là coûte un message clair à l'opérateur ; l'accepter coûterait un
segment qui se décode autrement selon qui le lit. L'aléa du parcours Last.fm
suit la même règle, pour la même raison, et se tire d'un générateur
cryptographique sur au moins cent vingt-huit bits : il est la seule chose qui
sépare un retour légitime d'un retour fabriqué.

**Pas de table, pas d'administration à chaud.** Une destination reste un
réglage de déploiement, pas une ligne que l'on ajoute en marche. Une table
demanderait de réconcilier la configuration et la base à chaque démarrage, et
d'inventer une réponse pour un lien dont la destination a disparu de l'une mais
pas de l'autre. Ajouter une instance coûte un redémarrage, ce qu'un serveur
auto-hébergé supporte. Le jour où WaveFlow aura une console d'administration,
cette colonne deviendra une table pour une raison qui se voit.

**Une destination retirée casse le lien, elle n'en glisse pas un autre.** Si la
configuration ne nomme plus `maloja/alice` au démarrage, les liens qui la
visaient passent `broken`, plus rien ne part, et rien n'est remappé vers une
autre instance du même destinataire. Faire glisser `alice` vers `default`
enverrait les écoutes d'une personne sur le profil d'une autre — la substitution
que la décision 4 construit tout un étage d'identifiants pour empêcher.
L'opérateur remet la destination, ou la personne délie et relie.

**Et ses lignes en attente finissent, elles ne patientent pas.** Un lien
`broken` dont la file reste `pending` est une file que rien ne videra jamais :
`pending` échappe à la rétention, donc la table garde ces lignes sans fin, et
`oldest_pending_at` tient le lien `degraded` pour toujours — un troisième
exemplaire du défaut que cette révision corrige déjà deux fois. Elles passent
donc dans un état terminal, comme le fait déjà le déliement à la décision 4 :
même situation, même règle, et la rétention peut alors faire son travail.

**Et cela se fait avant que le drainage ne démarre.** L'ordre est la moitié de
la règle : réconcilier les liens pendant que la tâche de fond tourne déjà laisse
une fenêtre, courte et suffisante, où des écoutes partent vers la destination
d'hier. `initialize` compare donc les identités et termine ce qui doit l'être
sous le verrou d'écriture, et `spawn_scrobble_drain` ne commence qu'ensuite.
Rien d'autre n'a besoin d'être gardé : une fois le lien `broken`, la mise en
file n'écrit plus rien pour lui.

**Un nom n'est pas une identité : l'URL en fait partie.** Retirer `maloja/alice`
casse ses liens, mais lui donner une autre URL les enverrait ailleurs sans que
rien ne change de nom — la même substitution, par la porte d'à côté, et la plus
facile à commettre puisqu'elle ressemble à une correction de configuration. Un
lien retient donc de quoi reconnaître la destination qu'il visait, empreinte de
l'URL comprise ; si elle ne correspond plus au démarrage, il passe `broken`
comme si le nom avait disparu.

**Ce qu'est cette empreinte, dit avant d'être écrite.** Elle porte sur l'URL
*normalisée*, pas sur la chaîne que l'opérateur a tapée : sans cela,
`https://maloja.example` et `https://maloja.example/` casseraient tous les
liens d'un serveur au premier redémarrage après qu'un éditeur eut ajouté une
barre.

**Et cette normalisation n'est pas celle de l'URL publique.**
`normalize_public_url` rend une origine — schéma, hôte, port — et jette le
chemin, parce qu'une origine est ce dont les partages ont besoin. Une
destination, non : `validate_destination` accepte un chemin, donc
`https://host/tenant-a` et `https://host/tenant-b` sont deux destinations, et
les réduire à leur origine leur donnerait la même empreinte. La garde
d'identité laisserait alors passer exactement le déplacement qu'elle existe
pour voir — d'un locataire à un autre sur la même machine, ce qui est la forme
la plus vraisemblable qu'un tel changement puisse prendre.

Il faut donc une canonicalisation propre aux destinations : schéma, hôte, port
et **chemin** sérialisés sans ambiguïté, la chaîne de requête et le fragment
refusés comme ils le sont déjà, et la barre finale tranchée plutôt que laissée
au lecteur — `https://host/maloja` et `https://host/maloja/` sont **la même**
destination, parce qu'un opérateur qui colle l'une ou l'autre veut la même
machine, et qu'un lien cassé par une barre ajoutée en éditant un fichier serait
une punition pour rien. La forme canonique est celle sans barre finale, la
racine mise à part, où il n'y a pas de choix à faire. La même normalisation sert
au rattrapage, à la réconciliation et à la garde de reprise : trois lectures
d'un même fait, qui ne valent que si elles le calculent pareil — et les deux
écritures d'une même URL sont ce qu'un test tient ensemble. Comparer des
empreintes plutôt que des URL n'est pas une précaution contre un attaquant —
personne d'hostile n'écrit cette configuration — mais la façon de n'avoir qu'une
seule chose à comparer.

C'est la décision 4 appliquée un étage plus haut. Là-bas, délier puis relier
crée une génération qui n'hérite de rien, parce que le compte et la destination
peuvent être les mêmes sans que l'autorisation le soit. Ici, le nom peut être le
même sans que la machine au bout le soit, et une file en attente ne doit pas
découvrir la différence en la franchissant. Corriger une coquille dans une URL
coûte alors de relier — le prix d'une distinction qu'aucune heuristique ne peut
faire à la place de l'opérateur.

## Ce que cette RFC change ailleurs

- **Une dépendance sortante entre dans `Cargo.toml`** pour la première fois.
  C'est le point le plus lourd de conséquences, et il mérite d'être dit plutôt
  que découvert dans un `cargo tree`.
- **Deux tables** : `scrobble_link` et `scrobble_outbox`, en migrations datées,
  comme toujours.
- **Une septième tâche de fond**, démarrée dans `src/main.rs` avec les six
  autres — puis une huitième, pour la purge, quand la rétention s'écrira.
- **Des colonnes de plus, en migrations datées comme les tables.** Sur
  `scrobble_outbox`, `retried_at` — l'invariant de la décision 13 ne peut pas se
  déduire d'une jointure que la purge dénoue — et l'instant de passage en état
  terminal sur lequel la rétention compte. Sur `scrobble_link`, le nom de la
  destination et l'empreinte de son URL.

  **Et `retried_at` se remplit lui aussi, sur les bases déjà en service.** Des
  entrées retentées existent depuis [#191](https://github.com/InstaZDLL/waveflow-server/pull/191),
  reconnaissables à la jointure que cette révision abandonne. Ajouter la colonne
  vide les rendrait toutes répondables une seconde fois : la migration
  réintroduirait, chez les seuls serveurs déjà en route, exactement le défaut
  qu'elle vient corriger. Elle lit donc `retry_of` une dernière fois pour écrire
  la colonne, et prend pour instant celui où la ligne de reprise fut créée, qui
  est le moment où le joker a été consommé. Celle-là tient en SQL — aucune
  configuration n'entre dans ce calcul — et un test sur une base peuplée avant
  la colonne est ce qui le prouve, pas une relecture.
- **Et un rattrapage, qui est la partie qu'on oublie.** Les liens déjà écrits ne
  portent ni nom ni empreinte, et il n'existe qu'une destination par destinataire
  au moment où cette révision s'écrit : ils reçoivent celle-là, avec l'empreinte
  de l'URL alors configurée. Une valeur nulle laissée en place reviendrait à
  traiter tout lien ancien comme une destination disparue, et à casser au
  démarrage suivant des liens que rien n'a déplacés.

  **L'instant terminal se rattrape lui aussi**, et sur la même base déjà en
  service : les lignes `sent`, `rejected`, `abandoned`, `cancelled` et
  `discarded` écrites avant la colonne n'en portent aucun, et la purge ne
  saurait pas à partir de quand les compter. Elles reçoivent leur `updated_at`,
  qui est l'écriture qui les a rendues terminales — rien ne les a touchées
  depuis. Les `uncertain` n'en reçoivent pas : elles ne se purgent pas, et leur
  en donner un inviterait quelqu'un à s'en servir un jour pour les compter. Cela
  s'éprouve comme le reste, sur une base peuplée avant la colonne : chaque état
  terminal rattrapé, les `uncertain` laissées vides, et un instant terminal
  qu'une écriture ultérieure ne rebouge pas.

  **Pourquoi `updated_at` fait l'affaire ici**, alors qu'il ne la ferait pas pour
  la suite. Les huit `UPDATE` que connaît `scrobble_outbox` exigent tous un état
  de départ non terminal — `pending`, `sending` ou `uncertain` — dans leur
  `WHERE`. Rien n'écrit sur une ligne déjà `sent`, `rejected`, `abandoned`,
  `cancelled` ou `discarded` : sa dernière écriture *est* sa transition, et il
  n'existe donc pas de ligne historique dont l'instant serait indéterminable.
  C'est une propriété du code d'aujourd'hui, pas une garantie du schéma, et
  c'est exactement pourquoi la colonne existe pour la suite plutôt que de
  continuer à lire `updated_at` : le premier `UPDATE` écrit sans garde d'état la
  ferait tomber sans bruit.


  **Ce rattrapage affirme, il ne vérifie pas.** Il ne peut pas : rien en base ne
  dit quelle URL était configurée hier, donc écrire l'empreinte courante revient
  à déclarer que la destination d'aujourd'hui est bien celle que ces liens
  visaient. C'est vrai tant qu'il n'y a qu'une réponse possible, et c'est le cas
  au moment où ceci s'écrit. Si des liens hérités existent chez un destinataire
  et que la configuration en déclare déjà plusieurs pour lui, il n'y a pas de
  réponse : le serveur refuse de partir plutôt que d'en choisir une, parce que
  se tromper ici enverrait des écoutes en attente sur le mauvais profil sans que
  rien ne le signale. La sortie ne demande aucun mécanisme neuf : démarrer une
  fois avec une seule destination par destinataire — celle que ces liens
  visaient — remplit les colonnes, et les autres s'ajoutent au démarrage
  suivant. Le rattrapage n'a lieu qu'une fois ; ce qui vient après passe par la
  réconciliation ordinaire.

  **Rien de tout cela ne concerne une installation neuve.** Le refus porte sur
  l'ambiguïté d'un rattrapage, et sans lien hérité il n'y a rien à rattraper :
  une base vide démarre avec autant de destinations que l'opérateur en déclare,
  dès la première fois. Ne pas le dire aurait rendu la fonctionnalité
  inatteignable à ceux qui n'ont rien à migrer — un garde-fou qui ferme la porte
  qu'il protège.

  **Et si un destinataire a des liens sans qu'aucune destination ne soit déclarée
  pour lui** — une URL vidée de la configuration, ce qui est déjà une façon de le
  désactiver — il n'y a rien à inscrire : ces liens sont traités comme ceux d'une
  destination disparue, cassés et leur file terminée, plutôt que laissés avec
  deux colonnes vides que la réconciliation suivante ne saurait pas lire.

  **Conséquence de déploiement, dite plutôt que tue.** Qui change l'URL d'une
  destination *puis* met à jour verra le rattrapage entériner ce changement :
  l'empreinte inscrite sera la nouvelle, et les écoutes en attente partiront
  vers la nouvelle machine sans que la garde d'identité, qui n'a pas connu
  l'ancienne, ait rien à signaler. C'est le seul instant du cycle où cette garde
  ne peut pas jouer, et c'est dans sa nature : elle compare à ce qui a été
  inscrit, et ici rien ne l'a été. L'ordre à tenir est donc l'inverse — mettre à
  jour d'abord, changer l'URL ensuite, et la garde fera son travail. Un opérateur
  qui a déjà fait l'autre délie avant de mettre à jour.

  **Le SQL ne peut pas le faire.** `Database::migrate` n'a que la base ;
  l'empreinte se calcule sur une URL qui vient de `Config::from_env`, que la
  migration ne voit pas. Celle-ci ajoute donc les colonnes, nullables, et
  `initialize` les remplit — au même endroit et sous le même verrou que la
  réconciliation décrite plus haut, dont ce rattrapage est le premier tour :
  remplir puis comparer, une fois, avant que `spawn_scrobble_drain` ne parte.
- **La documentation** : une section du guide d'API, et une ligne dans
  `docs/web-client-gap-analysis.md`, dont le point 15 attend celle-ci.

## Décision 11 — l'ordre des destinataires, et une écoute par requête

**ListenBrainz, puis Maloja, puis Last.fm.**

ListenBrainz épouse l'architecture ci-dessus sans rien lui demander : un jeton
par personne, du JSON, une soumission permanente et un `playing_now` que sa
documentation donne explicitement pour temporaire — la décision 3 exactement.
Il s'auto-héberge, donc il éprouve aussi la décision 10 dès le premier jour.

Maloja vient ensuite : son API native accepte directement une liste d'artistes,
un artiste d'album, une durée et un horodatage, donc l'enveloppe de la
décision 2 s'y verse sans perte.

Last.fm en dernier, **parce que c'est lui qui éprouve le plus l'abstraction** :
signature, identifiants d'application en plus de la session du compte, et des
erreurs applicatives sous un `200`. Un adaptateur écrit pour lui d'abord aurait
fait fuir ses particularités dans le drainage ; écrit en troisième, il se
heurte à une frontière déjà tenue par deux autres.

### Une écoute, une requête

Last.fm accepte jusqu'à cinquante scrobbles par appel. **On n'en profite pas,**
**pas au début.** Une requête de cinquante qui revient `Ambiguous` rend
cinquante écoutes incertaines d'un coup, et la décision 5 interdit alors de
retenter : le lot transforme une perte possible en cinquante pertes probables.

Une ligne de file, une requête. Grouper redeviendra une décision le jour où le
débit sera un problème mesuré, et ce jour-là il faudra dire ce qu'un lot
ambigu devient — ce que cette RFC n'a pas à trancher pour un serveur personnel.

### Last.fm ne se colle pas, il s'autorise

**Ajouté le 2026-09-14.** Toute la surface de la décision 9 tient dans un geste :
présenter un secret déjà en main à `PUT /api/v2/scrobble-links/{provider}/{destination}`.
Last.fm n'en délivre pas. Il faut un aller-retour par le navigateur de la
personne — un jeton de requête, une autorisation chez eux, puis la conversion du
jeton en clé de session, laquelle dure jusqu'à révocation.

**Le serveur porte ce parcours.** L'autre voie — « obtenez une clé de session
avec un outil tiers, puis collez-la » — était tentante parce qu'elle ne coûte
rien : c'est aussi ce qui la disqualifie. Elle sort une seule destination du
modèle où vivent les deux autres, et fait payer à la personne la particularité
d'un fournisseur. Elle reste bonne pour un diagnostic, pas comme chemin normal.

Deux routes, et un état temporaire qui n'entre pas dans `scrobble_outbox` :

- `POST /api/v2/scrobble-links/lastfm/authorize/{destination}` ouvre un état lié
  au compte et rend l'URL où envoyer la personne : `/api/auth` chez Last.fm,
  portant la clé d'application et un `cb` qui désigne la route ci-dessous.
- `GET /api/v2/scrobble-links/lastfm/callback/{state}` reçoit le `token` que
  Last.fm y ajoute, vérifie l'état que porte son chemin, appelle
  `auth.getSession` pour échanger le jeton contre la clé de session, la scelle
  et crée le lien.

**La destination se nomme à l'aller et se retrouve au retour.** Last.fm n'ayant
qu'une instance, il serait tentant de la sous-entendre — mais cette section
vient de refuser tout défaut implicite, et une exception pour un destinataire
serait la première marche vers le glissement qu'elle interdit. Le nom est donc
demandé, vérifié contre les destinations déclarées, et rangé dans l'état : c'est
lui que le retour relira, plutôt que d'en deviner un. Le jour où quelqu'un
déclare deux Last.fm — un compte de famille et le sien — rien n'aura à changer.

**Et le retour revérifie la destination.** Un quart d'heure sépare l'aller du
retour, et un serveur peut redémarrer entre les deux — c'est justement pour
survivre à cela que l'état vit en base. Il porte donc aussi l'empreinte de la
destination au moment de l'aller, et le retour la compare à celle d'après
redémarrage avant d'appeler `auth.getSession` : destination disparue ou
déplacée, le retour est refusé et rien n'est créé. Sans quoi le parcours
fabriquerait un lien vers une machine que la personne n'a pas choisie — ce que
la garde d'identité empêche partout ailleurs, et qui serait ici plus fâcheux
puisque le lien naîtrait déjà faux.

**Le littéral vient avant le paramètre, et ce n'est pas une préférence de
style.** Si la route s'écrivait `…/lastfm/{destination}/authorize` — et c'est
précisément pourquoi elle ne s'écrit pas ainsi — une destination nommée
`callback` produirait `…/lastfm/callback/authorize`, que le retour
`…/lastfm/callback/{state}` réclame tout autant — deux motifs de même forme, et
un routeur qui doit trancher. Réserver le mot marcherait ; le déplacer supprime
la question. Une liste de noms interdits est une chose qu'il faut se rappeler de
tenir à jour, et elle s'oublie au premier segment qu'on ajoute.

**C'est le parcours web, et il n'appelle pas `auth.getToken`.** Cette méthode
appartient au parcours des applications de bureau, où le jeton se demande avant
d'ouvrir la page ; ici c'est Last.fm qui le remet, sur le retour. Les deux se
ressemblent assez pour se mélanger — la première rédaction de ce paragraphe les
avait mélangés — et une implémentation qui demanderait un jeton puis en
recevrait un autre travaillerait avec le mauvais.

L'état est à usage unique et **expire en dix à quinze minutes**, donc bien avant
les soixante que Last.fm accorde à son jeton : c'est nous qui refusons en
premier, et jamais un jeton périmé qui nous surprend. Il porte de quoi se
reconnaître sans rien deviner : un compte, un aléa, une échéance.

**L'aléa voyage dans le chemin, et non dans une chaîne de requête.** Last.fm
annonce qu'il remet le jeton en accolant `/?token=…` à la callback, ce qui ne
dit rien de ce qu'il ferait d'une callback portant déjà un `?`. Le `cb` désigne
donc `…/callback/{state}`, qui ne dépend d'aucune supposition sur cette
concaténation.

**Ce que le retour refuse avant d'appeler quoi que ce soit.** Un `token` absent,
vide, ou présent deux fois ; et le cas où Last.fm revient en disant qu'il a
échoué. Dans chacun, rien n'est échangé et aucun lien n'est créé — l'appel à
`auth.getSession` vient après ces refus, pas avant. Le cas répété mérite d'être
nommé : `?token=a&token=b` laisse un extracteur choisir, et un parcours dont
l'issue dépend de quel doublon un lecteur retient n'est pas un parcours. Le
refus ne journalise pas davantage que le succès.

**La barre finale n'est pas un détail de rédaction.** Last.fm annonce qu'il
accole `/?token=…`, donc ce que le navigateur demandera est
`…/callback/{state}/` et non `…/callback/{state}`. `axum` ne normalise pas les
barres finales : une route déclarée sans elle ne répondrait pas, et le parcours
échouerait à son dernier pas, chez tout le monde, pour un caractère. La route
accepte donc les deux formes, et un test le tient — pas une relecture attentive.

**Et l'état tient à deux choses, pas à une.** Le lier au seul compte suffirait
si l'URL de retour ne sortait jamais du navigateur qui l'a demandée — or elle
passe par Last.fm, et une URL qui voyage se retrouve dans un référent ou un
historique. Qui l'obtiendrait pourrait terminer le parcours avec *son* jeton, et
le compte de quelqu'un d'autre se mettrait à scrobbler sur son profil à lui.
L'état s'apparie donc au cookie décrit plus bas, le retour exige les deux, et il
se consomme avant l'échange du jeton plutôt qu'après la création du lien : ce
qui a servi une fois ne peut pas resservir, même si le premier essai échoue plus
loin.

**Ce jeton arrive dans une URL, et une URL se garde.** Il vaut soixante minutes
et vaut un profil : il n'a rien à faire dans un journal, un historique de
navigateur ni un référent. Trois conséquences pour cette route, aucune
facultative. Son chemin rejoint les préfixes que `trace_path` rédige, aux côtés
des billets de flux et des jetons de partage, ce qui couvre le `{state}` ; le
`token`, lui, est dans la chaîne de requête, qu'aucune trace de ce serveur ne
retient déjà. La règle de `CLAUDE.md` ne souffre pas d'exception pour un secret
d'une heure. La réponse porte `Cache-Control: no-store` et
`Referrer-Policy: no-referrer`, sans quoi le jeton voyagerait vers la page
suivante dans un en-tête que personne ne relit. Et elle redirige aussitôt vers
une adresse sans jeton, pour que ce soit celle-là que l'historique retienne — la
page où la personne atterrit n'a pas besoin d'en
savoir plus que « c'est lié ».

**Et le journal d'accès du proxy n'est pas le nôtre.** Ce serveur ne retient
aucune chaîne de requête, mais un reverse-proxy en écrit une par défaut, et le
jeton s'y retrouverait entier. L'opérateur qui en pose un devant retire ou
masque la chaîne de requête sur ce chemin — sans la retirer de la requête
transmise, que le gestionnaire lit. Cela ne se vérifie pas d'ici, comme
l'enregistrement de la callback ; cela se dit.

**Les états vivent en base et se purgent.** Les garder en mémoire les perdrait
au redémarrage, au milieu du seul parcours qui ne supporte pas d'être repris.
Une table minuscule, donc, avec une échéance — et la tâche de purge de la
rétention y passe aussi, puisqu'elle existera de toute façon. Elle y applique
l'échéance de chaque état et rien d'autre : la fenêtre de trente jours gouverne
la file, pas un parcours qui dure un quart d'heure, et emprunter la mauvaise
des deux garderait un état bien après qu'il aurait dû mourir. Une révision qui
tranche la croissance sans fin d'une table ferait mauvaise figure en en
introduisant une autre.

**La destination de retour se dérive de `WAVEFLOW_PUBLIC_URL`**, qui dit déjà
l'origine extérieure du serveur pour les partages. Un réglage de plus pour la
même chose serait une seconde vérité à tenir d'accord avec la première.

**Le retour n'arrive pas avec un jeton de porteur, et pas non plus avec la
session ordinaire.** C'est un navigateur qui revient de chez Last.fm, pas un
client qui appelle l'API : aucun en-tête `Authorization` ne l'accompagne. Et la
session web ne suffit pas davantage, ce que la première rédaction de ce
paragraphe affirmait à tort — les cookies de ce serveur sont tous
`SameSite=Strict` (`src/api/web_session.rs`), donc un retour venu d'un autre
site n'en porte aucun. Le parcours aurait échoué chez tout le monde, à son
dernier pas, pour une raison invisible à la lecture.

Ce qui autorise le retour est donc un **cookie propre à ce parcours**, posé par
`authorize` et attendu par lui seul : `HttpOnly`, `Secure`, `Path` réduit au
chemin du retour, et `SameSite=Lax`, qui est exactement ce qu'il faut — une
navigation `GET` de premier plan le porte, une requête de fond d'un autre site
ne le porte pas. Il vaut ce que vaut l'état, dure aussi peu, et se jette avec
lui. Les cookies de session ne bougent pas : `Strict` est le bon réglage pour
eux, et cette exception ne les concerne pas.
**Un parcours à la fois.** Le cookie porte un nom fixe, donc en ouvrir un second
remplace le premier : l'onglet resté en arrière ne pourra plus conclure, et son
état expirera seul. C'est la limite retenue plutôt que corrigée — lier son
compte à un service est un geste qu'on fait une fois, et un cookie par état
échangerait cette petite surprise contre un mécanisme dont personne n'a besoin.
Ce qu'il ne faut pas, c'est qu'un retour trouve un cookie qui ne lui correspond
pas et l'accepte quand même : les deux se vérifient ensemble, ou le retour est
refusé.

**Ce que RFC-002 demande, le retour y répond autrement.** La règle est qu'une
route ne peut pas exister sans dire quel `Access` elle exige. `authorize` y
répond comme les autres : le compte courant, et `Access::Write`, puisque poser
une autorisation est une mutation. Le retour, lui, n'en exige aucun, et ce
n'est pas un oubli : il n'est pas appelé par un client mais par le navigateur
d'une personne au retour d'un parcours qu'elle vient
d'ouvrir. Ce qui tient lieu de preuve est plus étroit qu'un `Access::Write` —
un aléa à usage unique, une échéance de quelques minutes, un cookie qui ne vaut
que pour ce chemin, et les trois doivent concorder. L'exception est nommée ici
plutôt que découverte dans un `authenticated` manquant.

**Et cette route veut `https`.** La décision 10 tolère le clair vers une cible
que l'opérateur déclare sur son propre réseau ; l'échappatoire ne vaut pas ici,
puisque le trajet qui rapporte le jeton part d'Internet et traverse le
navigateur d'une personne. Un `WAVEFLOW_PUBLIC_URL` en `http` devra donc laisser
Last.fm indisponible, au même titre qu'une valeur absente, et pour une raison
que la liste des destinations dira aussi clairement.

C'est une exigence à écrire, pas un état constaté : `normalize_public_url`
accepte aujourd'hui `http` comme `https`, et doit continuer — l'URL publique sert
aussi les partages, où le clair sur un réseau privé reste un choix d'opérateur
légitime. La condition appartient à l'activation de Last.fm, pas au réglage.

**Et cette URL s'enregistre chez eux.** Un compte d'application Last.fm porte
une callback, que le paramètre `cb` sert à remplacer pour une autorisation
donnée ; leur documentation ne dit pas si cette valeur est vérifiée contre le
domaine enregistré. L'opérateur y inscrit donc l'URL que son
`WAVEFLOW_PUBLIC_URL` produit, plutôt que de parier sur une tolérance que
personne ne lui a promise. C'est un prérequis de déploiement au même titre que
la clé d'application, et il vit au même endroit — rien ici ne peut le vérifier
depuis ce côté-ci.

Il en découle que **Last.fm est indisponible tant que `WAVEFLOW_PUBLIC_URL`
n'est pas configuré** — le serveur avertit déjà à ce sujet au démarrage. Ce
n'est pas un défaut à contourner mais une condition à dire : la liste des
destinations l'annonce, avec sa raison, plutôt que de laisser un lien échouer
plus tard sans explication.

**Pas de parcours sans navigateur dans ce lot.** Last.fm en documente un pour
les applications de bureau, et il pourra venir. Deux parcours écrits ensemble,
c'est deux fois l'occasion de se tromper sur l'état temporaire, et la CLI n'est
ici que pour l'opérateur qui prépare un serveur.

### Il est venu, le 2026-09-15, et il n'a pas d'état temporaire

Le report ci-dessus tenait sur une crainte nommée : *deux fois l'occasion de se
tromper sur l'état temporaire*. Celui-ci n'en a aucun. Le jeton de requête
appartient à Last.fm, il expire sur leur horloge, et **c'est la personne qui le
porte** d'une commande à l'autre. Pas de table, pas de cookie, pas d'échéance à
nous. Il n'y avait donc pas deux états à tenir, mais un seul — et c'est ce qui
a permis de l'écrire en second sans rejouer la crainte.

Deux commandes, là où le web a deux routes :

- `waveflow scrobble authorize --username … --destination …` appelle
  `auth.getToken`, imprime l'adresse à ouvrir et le jeton à rapporter. **Elle
  n'écrit rien** : une autorisation que personne ne termine ne coûte rien au
  compte, et aucune ligne ne traîne qu'une purge devrait ramasser.
- `waveflow scrobble exchange --username … --destination …` échange le jeton
  approuvé contre la clé de session et crée le lien.

**Il ne demande pas `WAVEFLOW_PUBLIC_URL`**, et c'est sa raison d'être. Le
parcours web l'exige parce que Last.fm doit ramener un navigateur à une adresse
que ce serveur répond ; ici rien ne revient. La seule condition est donc une
application — ce qui fait de ce parcours celui d'un serveur sans façade
publique, exactement la machine où la CLI est déjà le seul geste possible.

**Le jeton sort par une variable d'environnement, pas par `argv`**, comme le
secret de `link` et pour la même raison : un historique de shell et une liste de
processus se lisent. Le sien mérite ce soin autant qu'une clé de session — qui
l'échange le premier est celui à qui la session finit par appartenir.

**Et `auth.getToken` n'entre pas dans le parcours web pour autant.** Le
paragraphe ci-dessus disait que les deux se ressemblent assez pour se mélanger ;
maintenant que les deux existent, cette phrase cesse d'être un avertissement et
devient vérifiable. Le double de test compte les demandes de jeton, et le
parcours web en fait **zéro** — un serveur qui minterait le sien puis en
recevrait un autre travaillerait avec le mauvais, et c'est désormais une
assertion et non une intention.

## Décision 12 — ce que l'API montre : des compteurs, jamais un écho

Un lien expose son état et la forme de sa file, agrégés :

- `healthy`, `degraded`, `broken` ;
- combien de lignes attendent, combien retentent, combien sont `uncertain` ;
- depuis quand attend la plus ancienne, et quand remonte le dernier succès ;
- éventuellement une dernière cause normalisée — `rate_limited`, `auth_broken`.

**`healthy` ne peut pas vouloir dire « j'ai encore un jeton ».** Un lien valide
avec trois mille écoutes en attente depuis six heures est en panne, et c'est
précisément la panne silencieuse qu'une file durable existe pour rendre
visible. D'où `degraded` : le lien répond, mais la file ne se vide pas — une
incertaine qui attend qu'une personne réponde d'elle, ou une entrée en attente
depuis plus longtemps que le seuil. `broken` reste réservé à `AuthBroken` et à
une configuration inutilisable.

**Corrigé le 2026-09-14 :** cette phrase disait « des reprises, des incertaines,
ou une attente trop vieille », et `link_health` ne regarde pas les reprises. Il
avait raison de ne pas le faire — une ligne qui retente est une file qui
fonctionne, et la compter dégraderait tout lien ayant croisé une panne
passagère. C'est le texte qui promettait de travers, depuis l'origine. L'ordre
des trois questions vit dans `link_health` et n'est pas recopié ici : deux
écritures du même seuil divergeraient, et c'est le code que l'API rend.

**Retiré le 2026-09-14 :** une révision de ce document a prétendu ici que « quand
remonte le dernier succès » n'était tenu par rien, et qu'il faudrait un jour une
colonne sur `scrobble_link` écrite au moment du succès. Cette colonne existe
depuis [#191](https://github.com/InstaZDLL/waveflow-server/pull/191) :
`scrobble_link.last_success_at`, écrite dans la transaction même qui règle une
ligne, publiée par `scrobble_links`. La forme prescrite était déjà la forme en
place.

L'erreur vaut d'être gardée parce qu'elle a une cause nette : `link_health` ne
prend pas ce champ, et j'en ai conclu qu'il n'existait pas. Il ne le prend pas
parce que la santé n'en dépend pas — un lien qui n'a jamais rien envoyé n'est
pas malade — et le champ voyage à côté, dans `ScrobbleLinkState`. **Lire la
signature d'une fonction ne dit pas ce qu'une structure porte.**

**Jamais le contenu de l'enveloppe, jamais la réponse brute du fournisseur.**
La décision 10 interdit déjà l'écho ; ceci en est le corollaire du côté
lecture. Ce qui sort est un compte, pas un titre ni un corps d'erreur.

**Et cela ne passe pas par la synchronisation.** C'est de l'état opérationnel
du serveur, pas une donnée d'utilisateur à répliquer vers le desktop : la
[RFC-003](RFC-003-waveflow-sync-v2.md) n'a pas à le porter, et une lecture
native du lien suffit. L'y ajouter ferait voyager, à chaque appareil et à
chaque réveil, un compteur qui ne décrit qu'une machine.

## Décision 13 — `uncertain` se jette ou se retente, et le doublon est choisi

Une entrée `uncertain` est terminale pour le serveur. La personne, elle, a deux
gestes : **`discard`** et **`retry`**.

C'est la seule forme de doublon que cette RFC accepte, parce qu'il est choisi.
Le geste porte donc son avertissement : *le destinataire a peut-être déjà
enregistré cette écoute ; retenter peut la compter deux fois.* Une personne qui
tient à son historique préférera parfois le trou, et c'est son arbitrage, pas
celui du serveur.

**Retenter n'est pas réactiver.** La tentative ambiguë reste dans l'histoire
telle qu'elle fut ; la nouvelle s'y ajoute, marquée comme demandée par la
personne. Effacer la première ferait mentir la seule trace qui explique
pourquoi un doublon existe chez le destinataire.

**Une entrée à la fois, et pas de « tout retenter ».** Un bouton qui relance
trois cents incertaines transforme une décision consciente en accident. Il
pourra s'ajouter plus tard, si quelqu'un le demande en sachant ce qu'il
demande.

### Le joker se compte sur l'entrée, pas sur sa descendance

**Révisé le 2026-09-14.** La première écriture déduisait « déjà retentée » d'une
jointure : une entrée était encore répondable tant qu'aucune autre ligne ne la
désignait par `retry_of`. C'est faux dès qu'une ligne peut disparaître, et la
rétention ci-dessous fait exactement disparaître des lignes — la reprise finit
`sent`, donc purgeable, et l'original redevient alors répondable. Mesuré plutôt
que craint : la reprise supprimée, l'entrée reparaît dans
`uncertain_scrobbles`, `retry_uncertain_scrobble` l'accepte une seconde fois, et
le compteur `uncertain` du lien le repasse `degraded` pour une écoute déjà
répondue.

Le joker devient donc un fait porté par l'entrée elle-même : **`retried_at`**,
nul jusqu'à ce qu'il ne le soit plus. La reprise s'écrit en une transaction,
l'UPDATE d'abord :

```sql
UPDATE scrobble_outbox SET retried_at = ?
 WHERE id = ? AND state = 'uncertain' AND retried_at IS NULL;
-- exactement une ligne modifiée, sinon le geste est refusé
INSERT INTO scrobble_outbox (..., retry_of, ...) VALUES (..., ?, ...);
```

L'ordre compte : c'est l'UPDATE conditionnel qui arbitre, et deux demandes
simultanées ne peuvent pas consommer deux fois le même joker. `retry_of` et son
index unique restent — ils disent la provenance, et refusent l'insertion en
second rempart — mais plus rien d'observable ne dépend de la survie de la ligne
qu'ils désignent.


**Une reprise vérifie aussi où elle irait.** `retry_uncertain_scrobble` exige
déjà un lien `active` — une écoute mise en file sous une autorisation retirée
partirait sinon sur le profil de la suivante, ce que la décision 4 empêche. La
règle d'identité ci-dessus ajoute la seconde moitié : l'empreinte de l'URL doit
encore correspondre. Sans elle, l'entrée la plus dangereuse qui soit — une
écoute que la personne accepte de risquer en double — serait celle qui part sur
une machine que personne n'a choisie. La vérification précède l'UPDATE, dans la
même transaction que lui.

**Pas d'état `resolution` à côté de `state`.** Jeter écrit déjà
`state = 'discarded'` sur l'entrée : une énumération qui redirait `discarded`
placerait deux sources de vérité sur le même fait. Seule la reprise laisse
l'entrée en `uncertain`, et seule elle a besoin d'être notée.

**Et tout ce qui lisait la jointure lit désormais la colonne.** Ce paragraphe a
d'abord dit *trois* endroits — la liste des incertaines, le compteur du lien et
le refus d'une seconde reprise. Ils sont **quatre** : le refus d'un *rejet*
aussi, `discard_uncertain_scrobble`, qui porte la même exclusion depuis
[#193](https://github.com/InstaZDLL/waveflow-server/pull/193). Le compte venait
d'une relecture de la décision plutôt que du fichier ; c'est le fichier qui a
raison. Les quatre passent à `retried_at IS NULL`. Le compteur surtout :
`link_health` rend `degraded` dès qu'une incertaine est comptée, donc en
oubliant un seul de ces quatre endroits on obtient un lien définitivement en
peine à cause d'une écoute à laquelle sa personne a déjà répondu — la panne que
la décision 12 veut rendre visible, retournée en fausse alerte permanente.
Chacun des quatre est éprouvé à part, par une inversion qui ne remet que
celui-là sur la jointure et fait tomber sa seule assertion.

## La rétention de la file

**Tranchée le 2026-09-14.** La question n'était pas posée par la première
version de cette RFC : la file ne se purge jamais, donc la table grandit d'une
ligne par écoute et par destination, sans fin, chez un auditeur actif. Cela n'a
pas retenu [#191](https://github.com/InstaZDLL/waveflow-server/pull/191), et
n'appelle pour elle-même qu'un réglage de déploiement et une tâche de purge, sur
le patron de [RFC-007](RFC-007-library-event-stream.md) et la huitième tâche de
fond. Les colonnes que cette révision ajoute viennent d'ailleurs, et sont
énumérées plus bas.

**Sur quel instant on purge.** Ni `created_at`, qui date la mise en file et ferait
partir une ligne trente jours après avoir été écrite quelle que soit la date où
elle a fini, ni un `updated_at` dont rien ne garantit qu'il ne rebougera plus.
Une ligne retient l'instant où elle est devenue terminale, une fois, et la purge
compte à partir de là. C'est le genre de propriété qui se perd entre une RFC et
un `UPDATE` de plus, donc elle est écrite : cet instant ne se réécrit pas.

- **Trente jours pour les états terminaux ordinaires** — `sent`, `rejected`,
  `abandoned`, `cancelled`, `discarded`. Assez pour comprendre une panne
  passée, trop court pour que la file devienne un second historique d'écoute.
  C'est le défaut d'un réglage, pas une durée gravée : qui diagnostique une panne
  de trois mois l'allonge, comme il règle la fenêtre de RFC-007.
- **`uncertain` se garde indéfiniment**, qu'il porte `retried_at` ou non. Une
  entrée sans réponse attend une personne, et la lui retirer au bout d'un mois
  serait décider à sa place. Une entrée répondue pourrait s'en aller plus tard ;
  ce n'est pas le moment de le décider, parce que `retry_of` est déclaré
  `ON DELETE SET NULL` et que l'index d'unicité `(play_event_id, link_id)` ne
  vaut que `WHERE retry_of IS NULL` — supprimer l'original dénullifie la reprise
  et la fait retomber sous cet index. Cela s'éprouve, et le gain se compte en
  kilo-octets.
- **Aucun plancher**, contrairement à RFC-007. Là-bas le flux sert une reprise
  de synchronisation et couper la tête d'une bibliothèque tranquille renverrait
  quelqu'un à l'instantané. Ici, personne ne reprend un curseur : l'outbox est
  un moyen de livraison, pas une source de vérité. Ce que le lien publie —
  compteurs et santé — se lit sur les lignes vivantes, `pending` et `uncertain`,
  qu'aucune purge ne touche.
- **Ce que le lien publie ne dépend d'aucune ligne purgeable**, et c'est déjà
  vrai : les compteurs et l'attente la plus ancienne se lisent sur `pending` et
  `uncertain`, qu'aucune purge ne touche, et `last_success_at` est une colonne
  de `scrobble_link` écrite au moment du succès. Une révision de ce document a
  affirmé ici le contraire — que ce dernier champ n'existait pas et se
  calculerait en `MAX(updated_at)` sur des lignes `sent` — ce qui aurait été un
  vrai défaut si c'avait été vrai. Ce ne l'était pas : la colonne est là depuis
  #191. Ce qu'il reste de la mise en garde, et qui vaut : rien de ce qu'un lien
  publie ne doit se mettre à dépendre des lignes que la purge emporte.

**Ce que cette borne ne borne pas, et pourquoi on l'accepte.** Une destination
qui répond ambigu à chaque envoi produit une `uncertain` par écoute, et aucune
ne s'en va : la table croît alors aussi vite que l'auditeur écoute, exactement
ce que la rétention corrige ailleurs. La limite est réelle et elle est retenue
telle quelle, parce que l'alternative demande de trancher ce qu'une entrée
devient quand personne ne répond — et la décision 13 refuse justement de
répondre à la place de quelqu'un. Ce qui rend le cas supportable est ailleurs :
un lien dans cet état est `degraded` dès la première ambiguïté, donc la panne
se voit avant que le volume ne compte. Si elle se voit sans être regardée, la
question reviendra, et elle reviendra avec une mesure plutôt qu'avec une
crainte.

## Ce qui reste ouvert

Plus aucune décision d'architecture, et plus aucune question de rétention. Ce
qui reste appartient à l'implémentation : la forme précise du JSON, et le seuil
au-delà duquel une file qui n'avance pas devient `degraded` — celui-là vit déjà
dans `link_health`.

**Les chemins écrits ci-dessus, eux, engagent.** La première version rangeait
« le nom exact des routes » parmi les détails, ce qui était vrai tant qu'aucune
route n'était décidée. Les décisions 10 et 11 en nomment désormais, et elles le
font parce que la forme *est* la décision : une destination dans le chemin
plutôt que sous-entendue, un aléa dans un segment plutôt que dans une chaîne de
requête. Les renommer se peut, les reformer non.

C'est la ligne *Implémentée par* de l'en-tête qu'il faut lire pour le reste.
