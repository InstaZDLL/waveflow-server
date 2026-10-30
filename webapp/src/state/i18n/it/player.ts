// The player bar, now playing, radio and offline listening.
import type { TranslationValue } from "..";
import type { player as en } from "../en/player";

export const player: Record<keyof typeof en, TranslationValue> = {
  "player.error":
    "Impossibile avviare la riproduzione. Controlla il server e riprova.",
  "player.previous": "Brano precedente",
  "player.play": "Riproduci",
  "player.pause": "Pausa",
  "player.next": "Brano successivo",
  "player.seek": "Posizione",
  "player.shuffle": "Riproduzione casuale",
  "player.repeatOff": "Ripeti: no",
  "player.repeatAll": "Ripeti: tutta la coda",
  "player.repeatOne": "Ripeti: questo brano",
  "player.volume": "Volume",
  "player.mute": "Disattiva audio",
  "player.unmute": "Riattiva audio",
  "player.openAlbum": "Apri l’album",
  "player.live": "In diretta",
  "player.liveRadio": "Radio in diretta",
  "player.stopLive": "Ferma la stazione",
  "player.liveError": "Impossibile riprodurre questa stazione.",
  "player.closeAside": "Ferma e chiudi",
  "playing.idle": "Nessuna riproduzione in corso.",
  "playing.empty": "Avvia un brano e comparirà qui, con il suo testo.",
  "playing.viewLabel": "Come visualizzare questo brano",
  "playing.viewDetail": "Dettagli",
  "playing.viewLyrics": "Testo",
  "playing.viewImmersive": "Immersiva",
  "playing.immersiveTitle": "Vista immersiva",
  "playing.leaveImmersive": "Esci dalla vista immersiva",
  "radio.detail": "Radio online, salvate con il tuo account",
  "radio.name": "Nome della stazione",
  "radio.address": "Indirizzo dello stream",
  "radio.homepage": "Pagina della stazione (facoltativa)",
  "radio.image": "Immagine della stazione (facoltativa)",
  "radio.imageHint":
    "Il server la scarica una sola volta, quando salvi, e la conserva con le copertine: il tuo browser non contatta mai il sito che ospita l’immagine.",
  "radio.imageMissing":
    "Non è stato possibile recuperare l’immagine; salva la stazione per riprovare.",
  "radio.add": "Aggiungi stazione",
  "radio.save": "Salva stazione",
  "radio.edit": "Modifica",
  "radio.replacing":
    "Il salvataggio sostituisce tutti i campi, quindi un indirizzo di pagina vuoto cancella quello salvato.",
  "radio.listen": "Ascolta",
  "radio.stop": "Ferma",
  "radio.visit": "Pagina della stazione",
  "radio.empty": "Ancora nessuna stazione. Aggiungine una per tenerla qui.",
  "radio.addressRefused": "L’indirizzo deve iniziare con http:// o https://.",
  "radio.mixedContent":
    "Questa pagina è servita in HTTPS, quindi il tuo browser rifiuterà uno stream non cifrato.",
  "radio.full":
    "Questo account ha già il numero massimo di stazioni consentito.",
  "radio.saveError": "Non è stato possibile salvare la stazione.",
  "radio.deleteError": "Non è stato possibile eliminare la stazione.",
  "radio.playbackError":
    "Impossibile riprodurre questa stazione. Il server non vi si collega mai, quindi la questione riguarda solo il tuo browser e la stazione.",
  "radio.stations": "Stazioni",
  "radio.newTitle": "Aggiungi una stazione",
  "radio.editTitle": "Modifica «{name}»",
  "radio.listenTo": "Ascolta {name}",
  "radio.stopNamed": "Ferma {name}",
  "radio.actions": "Azioni per {name}",
  "radio.confirmDelete":
    "Eliminare la stazione «{name}»? Il suo indirizzo e la sua immagine verranno dimenticati.",
  "offline.kicker": "In questo browser",
  "offline.title": "Salvati per l’ascolto offline",
  "offline.detail":
    "Questi brani sono stati salvati in questo browser per l’ultimo account che ha effettuato l’accesso, e si riproducono senza connessione. Uscendo, vengono eliminati da questo browser. Puoi salvarne altri dal menu di un brano.",
  "offline.unsupported":
    "Questo browser non può salvare brani per l’ascolto offline.",
  "offline.back": "Torna alla libreria",
  "offline.empty":
    "Non hai ancora salvato nulla. Il menu di un brano ti permette di salvarlo.",
  "offline.player": "Lettore offline",
  "offline.gone": "Quella copia non è più in questo browser.",
  "offline.forget": "Rimuovi la copia offline di {title}",
  "offline.forgetShort": "Rimuovi",
  "offline.forgetDetail": "Libera {size}",
};
