// Scrobbling.
import type { TranslationValue } from "..";
import type { scrobbling as en } from "../en/scrobbling";

export const scrobbling: Record<keyof typeof en, TranslationValue> = {
  "scrobbling.title": "Scrobbling externe",
  "scrobbling.detail":
    "Transmettre ce que vous écoutez ici à ListenBrainz, Maloja ou Last.fm.",
  "scrobbling.none":
    "Ce serveur ne propose aucune destination. C’est son exploitant qui en décide.",
  "scrobbling.provider.listenbrainz": "ListenBrainz",
  "scrobbling.provider.maloja": "Maloja",
  "scrobbling.provider.lastfm": "Last.fm",
  "scrobbling.instance": "instance « {name} »",
  "scrobbling.unavailable": "Indisponible ici : {reason}",
  "scrobbling.unavailable.no_application_configured":
    "aucune application Last.fm n’est déclarée sur ce serveur",
  "scrobbling.unavailable.browser_journey_needs_https":
    "le parcours par le navigateur exige que l’adresse publique de ce serveur soit en https — un exploitant peut encore lier un compte en ligne de commande",
  "scrobbling.unavailable.unknown": "ce serveur n’a pas dit pourquoi",
  "scrobbling.retired":
    "Ce serveur ne propose plus cette instance. Plus rien ne partira vers elle — vous pouvez encore retirer l’autorisation.",
  "scrobbling.notLinked": "Non liée.",
  "scrobbling.secret": "Clé d’API",
  "scrobbling.secretHint": "Prise sur votre compte chez cette destination.",
  "scrobbling.link": "Lier",
  "scrobbling.working": "En cours…",
  "scrobbling.linkError":
    "Cette autorisation a été refusée. Vérifiez la clé et recommencez.",
  "scrobbling.unlink": "Délier",
  "scrobbling.unlinkError": "Ce lien n’a pas pu être retiré.",
  "scrobbling.connect": "Se connecter à Last.fm",
  "scrobbling.connectDetail":
    "Last.fm ne délivre aucune clé à recopier : il vous interroge chez lui et vous ramène ici.",
  "scrobbling.connectError": "Le parcours Last.fm n’a pas pu s’ouvrir.",
  "scrobbling.returned": "Last.fm est liée : {name}.",
  "scrobbling.health.healthy": "Les écoutes partent",
  "scrobbling.health.degraded": "La file n’avance pas",
  "scrobbling.health.broken": "Rompu",
  "scrobbling.healthDetail.healthy": "Rien n’est bloqué ici.",
  "scrobbling.healthDetail.degraded":
    "Le lien répond, mais les écoutes ne passent pas — soit l’une attend votre réponse, soit la plus ancienne attend depuis trop longtemps.",
  "scrobbling.healthDetail.broken":
    "L’autorisation a été refusée, ou ce serveur ne joint plus cette instance sous le nom qui a servi à créer le lien.",
  "scrobbling.pending": {
    one: "{count} en attente",
    other: "{count} en attente",
  },
  "scrobbling.retrying": {
    one: "{count} en reprise",
    other: "{count} en reprise",
  },
  "scrobbling.uncertainCount": {
    one: "{count} attend votre réponse",
    other: "{count} attendent votre réponse",
  },
  "scrobbling.oldest": "La plus ancienne attend depuis {when}.",
  "scrobbling.lastSuccess": "Dernière acceptée {when}.",
  "scrobbling.lastFailure": "Dernier refus : {reason}.",
  "scrobbling.uncertain": "Des écoutes que vous seul pouvez trancher",
  "scrobbling.uncertainDetail":
    "La connexion a été coupée après leur départ : personne ne sait si la destination les a enregistrées. La renvoyer peut la faire figurer deux fois dans un historique que d’autres lisent ; la jeter laisse un trou. Ce serveur ne choisira pas à votre place, et il ne le demandera qu’une fois.",
  "scrobbling.uncertainNone": "Rien n’attend votre réponse.",
  "scrobbling.playedAt": "Écoutée {when}",
  "scrobbling.attempts": {
    one: "{count} tentative",
    other: "{count} tentatives",
  },
  "scrobbling.retry": "La renvoyer",
  "scrobbling.discard": "La jeter",
  "scrobbling.answered": "Répondu.",
  "scrobbling.answerError":
    "Cette écoute n’a pas pu être traitée. Rechargez la page avant de réessayer.",
  "scrobbling.unnamedTrack":
    "Absente de l’historique que ce compte conserve encore",
  "scrobbling.reason.rate_limited": "la destination a demandé du répit",
  "scrobbling.reason.retryable": "la destination ne l’a pas prise",
  "scrobbling.reason.auth_broken": "l’autorisation a été refusée",
  "scrobbling.reason.rejected": "la destination l’a lue et l’a refusée",
  "scrobbling.reason.destination_gone":
    "ce serveur ne joint plus cette instance sous ce nom",
  "scrobbling.reason.credential_unreadable":
    "ce serveur n’a pas pu lire l’identifiant enregistré",
  "scrobbling.reason.attempts_exhausted": "elle a été tentée trop de fois",
  "scrobbling.reason.unlinked": "le lien a été retiré",
  "scrobbling.reason.ambiguous": "la réponse n’est jamais arrivée",
  "scrobbling.reason.interrupted": "le serveur s’est arrêté en plein vol",
};
