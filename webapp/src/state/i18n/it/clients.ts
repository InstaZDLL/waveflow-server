// Registered clients, the Subsonic password and OAuth consent.
import type { TranslationValue } from "..";
import type { clients as en } from "../en/clients";

export const clients: Record<keyof typeof en, TranslationValue> = {
  "clients.title": "Client registrati",
  "clients.detail":
    "Dispositivi e script che hanno una credenziale per la tua libreria. Se ne revochi una, smette subito di funzionare.",
  "clients.name": "A cosa serve",
  "clients.register": "Registra",
  "clients.once":
    "Copiala ora. Viene conservato solo il suo hash, quindi è mostrata una sola volta e mai più:",
  "clients.revoke": "Revoca",
  "clients.none": "Nessun client ha una credenziale per la tua libreria.",
  "clients.error": "Non ha funzionato. Non è cambiato nulla.",
  "clients.registered": "registrato il {when}",
  "clients.used": "usato l’ultima volta il {when}",
  "clients.unused": "mai usato",
  "subsonic.title": "Password Subsonic",
  "subsonic.detail":
    "La password separata con cui accedono i client Subsonic. Cambiarla sostituisce anche la chiave API, così una chiave trapelata smette di funzionare.",
  "subsonic.set": "impostata il {when}",
  "subsonic.changed": "cambiata l’ultima volta il {when}",
  "subsonic.password": "Nuova password",
  "subsonic.rotate": "Cambiala",
  "subsonic.revoke": "Disattivala",
  "subsonic.confirm": "Sì, disattivala",
  "subsonic.once":
    "Copia ora questa chiave API. Viene conservato solo il suo hash, quindi è mostrata una sola volta e mai più:",
  "subsonic.gone":
    "I tuoi client Subsonic smetteranno di funzionare finché non la reimposterai in ciascuno di essi.",
  "subsonic.none":
    "Non hai una password Subsonic, quindi nessun client Subsonic può leggere la tua libreria. Chiedine una all’amministratore.",
  "subsonic.short": "Usa almeno 12 caratteri.",
  "subsonic.error": "Non ha funzionato. Non è cambiato nulla.",
  "credential.password": "Password Subsonic dedicata",
  "credential.rotate": "Rinnova la credenziale",
  "credential.copy": "Copia ora questa chiave API:",
  "credential.error": "Il rinnovo della credenziale non è riuscito.",
  "authorize.title": "Autorizzazione",
  "authorize.invalid":
    "Questo link di autorizzazione è incompleto o chiede di mandarti verso un indirizzo di cui non ci fidiamo.",
  "authorize.client": "Autorizza {client}",
  "authorize.detail":
    "Potrà sfogliare le tue librerie, riprodurre la tua musica e gestire le tue playlist, i preferiti e le valutazioni: tutto ciò che può fare questo account.",
  "authorize.redirect": "Ti stiamo riportando a {url}",
  "authorize.error":
    "L’applicazione ha inviato una richiesta di autorizzazione che non possiamo accettare.",
  "authorize.busy": "Autorizzazione in corso…",
  "authorize.approve": "Autorizza",
};
