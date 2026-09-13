# RFC-010 — Le scrobbling externe

- **Statut** : Proposed
- **Implémentée par** : rien encore. Quand du code existera, c'est cette ligne
  qui nommera les PR, et le champ *Statut* ci-dessus ne basculera pas — il ne
  bascule jamais dans ce projet.
- **Date** : 2026-09-13
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

## Décision 2 — une file durable, jamais un envoi direct

Une table `scrobble_outbox`, drainée par une tâche de fond de la forme des six
autres — `spawn_upload_sweeper`, `spawn_canvas_sweeper`, `spawn_artwork_sweeper`
et leurs voisines, toutes démarrées dans `src/main.rs`.

L'alternative — appeler le service distant pendant la requête — est écartée pour
trois raisons qui tiennent chacune seule : la requête d'un client attendrait un
tiers dont le serveur ne contrôle ni la latence ni la disponibilité ; un
redémarrage perdrait ce qui n'était pas parti ; et un service muet pendant une
journée ferait échouer des écoutes que le serveur a pourtant enregistrées
correctement.

**Ce que la file porte** : l'identifiant de la ligne `play_event`, le compte, la
destination, le nombre de tentatives, la date de la prochaine, et la dernière
erreur lisible. Pas une copie de la piste : la piste se relit, et deux modèles
d'une même chose finissent par se contredire.

## Décision 3 — on met en file les écoutes, pas les « en écoute »

`play_event.submission` distingue déjà les deux, et la distinction est la même
chez les destinataires : `updateNowPlaying` chez Last.fm, `playing_now` chez
ListenBrainz.

**Une soumission est mise en file. Un « en écoute » est envoyé au mieux, sans
file et sans reprise.** Un « en écoute » n'a de valeur que pendant qu'il est
vrai : le retransmettre dix minutes plus tard annoncerait une piste que
l'auditeur a quittée depuis longtemps. Le mettre en file reviendrait à garantir
la livraison d'une information périmée, ce qui est pire que de la perdre.

## Décision 4 — les identifiants sont par compte, chiffrés sous la clé d'instance

Le précédent existe et il est bon : le mot de passe Subsonic dédié est chiffré
par `SecretBox` en ChaCha20-Poly1305 sous la clé d'instance. Les jetons de
scrobbling suivent le même chemin, dans une table `scrobble_link` par compte et
par destination.

**Par compte, pas par déploiement ni par bibliothèque.** Une écoute appartient à
une personne ; un réglage global ferait scrobbler tout le monde sur le profil de
l'opérateur, et un réglage par bibliothèque poserait une question à laquelle
personne ne tient — sous quel profil compte une écoute dans une bibliothèque
partagée.

**Jamais relus en clair par l'API.** Comme pour les jetons d'API, on les
remplace, on ne les relit pas. La sauvegarde reste ce que `CLAUDE.md` dit déjà :
`data/waveflow.db` et `data/instance.key` vont ensemble, sans quoi ces jetons ne
sont plus déchiffrables — et c'est le comportement voulu.

## Décision 5 — au plus une fois, et l'ordre ne compte pas

**Un doublon est pire qu'une perte.** Une écoute perdue manque à un compteur ;
une écoute envoyée deux fois abîme un historique public que la personne tient
parfois depuis des années, et qu'elle ne peut pas corriger facilement.

Donc : une ligne de file est unique par `(play_event, destination)`, contrainte
tenue par le schéma et pas seulement par le code ; elle n'est supprimée qu'après
un accusé positif ou un refus définitif ; et une tentative en vol n'est jamais
doublée par une seconde.

**L'ordre n'est pas garanti et n'a pas à l'être** : chaque soumission porte son
propre `played_at`, que les trois destinataires acceptent. Sérialiser la file
par compte coûterait un blocage de tête de file pour une propriété que personne
n'observe.

## Décision 6 — les échecs ne se valent pas

Trois familles, trois conduites :

- **Réseau, 5xx, 429** : on retente, avec un recul croissant et une gigue, un
  nombre borné de fois. Le `Retry-After` d'un 429 est honoré quand il est là ;
  c'est la même règle que celle déjà écrite pour le transcodage saturé.
- **Identifiants refusés (401, 403)** : on arrête pour ce lien, on le marque
  rompu, et l'API le dit au compte concerné. Réessayer indéfiniment avec un
  jeton révoqué est du bruit chez le destinataire et une file qui ne se vide
  jamais.
- **Refus de la charge (4xx autre)** : la ligne part, avec un journal. Aucune
  reprise ne corrigera une piste que le destinataire n'accepte pas.

**Une file qui ne se vide pas est un défaut**, pas un état. Après les tentatives
bornées, la ligne est abandonnée et comptée, et ce compte est lisible.

## Décision 7 — ce que le serveur n'envoie pas

- **Rien de rétroactif à l'activation.** Brancher un compte n'envoie pas son
  historique : personne ne veut voir dix ans d'écoutes remonter d'un coup sur
  son profil, et un destinataire lit cela comme un abus.
- **Aucune piste qu'il ne sait pas nommer.** Sans titre ni artiste, la
  soumission est inutilisable et fausse les statistiques du destinataire.
- **Rien depuis un partage.** `/share/{token}` sert un visiteur sans compte ;
  il n'y a pas de profil à créditer.

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

Ce qui reste au déploiement : le délai d'attente sortant, le plafond de
tentatives, l'intervalle de drainage, et la base d'URL des destinations
auto-hébergées.

## Décision 10 — la surface sortante est bornée, et c'est la décision qui compte

C'est le vrai risque de cette RFC : un serveur qui appelle une URL est un serveur
qu'on peut faire appeler une URL. Maloja et ListenBrainz s'auto-hébergent, donc
il existera un champ d'URL, et il ne peut pas être libre.

- **HTTPS seulement**, sauf pour une cible explicitement déclarée par
  l'opérateur en clair sur son propre réseau.
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

- **Quel destinataire d'abord.** ListenBrainz est le plus simple — un jeton, une
  URL, un corps JSON — et Last.fm demande une signature et une session. Le
  premier n'engage pas le second.
- **Si la profondeur de la file se montre** dans l'API, ou seulement son état
  rompu ou sain.
- **Ce qu'on fait d'une écoute dont la piste a disparu** entre l'enregistrement
  et le drainage. La ligne de file survit-elle à la piste, ou part-elle avec
  elle ? La cascade actuelle dirait la seconde ; rien n'en dépend encore.
