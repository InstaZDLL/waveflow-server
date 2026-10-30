// Administration, scans, maintenance and missing files.
import type { TranslationValue } from "..";
import type { admin as en } from "../en/admin";

export const admin: Record<keyof typeof en, TranslationValue> = {
  "admin.title": "Amministrazione",
  "admin.detail": "Librerie, scansioni e account",
  "admin.libraries": "Librerie",
  "admin.libraryName": "Nome della libreria",
  "admin.libraryPath": "Percorso assoluto della cartella sul server",
  "admin.registering": "Aggiunta in corso…",
  "admin.register": "Aggiungi e scansiona",
  "admin.scan": "Scansiona ora",
  "admin.accounts": "Account",
  "admin.webPassword": "Password web, almeno 12 caratteri",
  "admin.creating": "Creazione in corso…",
  "admin.create": "Crea account",
  "admin.enable": "Attiva",
  "admin.disable": "Disattiva",
  "admin.currentAccount": "Non puoi disattivare l’account che stai usando",
  "admin.libraryError":
    "Non è stato possibile aggiungere il percorso della libreria.",
  "admin.accountError": "Non è stato possibile creare l’account.",
  "admin.statusError": "Non è stato possibile cambiare lo stato dell’account.",
  "admin.scanError": "Non è stato possibile avviare la scansione.",
  "admin.scanQueued": "Scansione {id} in coda.",
  "admin.initialScan": "Scansione iniziale {id} in coda.",
  "admin.scanning": "Scansione in corso",
  "admin.nowPlaying": "In riproduzione ora",
  "admin.nobodyListening": "Nessuno sta ascoltando in questo momento.",
  "admin.tokens": "Token API di {username}",
  "admin.tokenName": "A cosa serve?",
  "admin.tokenCreate": "Genera",
  "admin.tokenOnce": "Copialo ora. Viene mostrato una sola volta e mai più:",
  "admin.tokenError": "Non è stato possibile generare il token.",
  "admin.tokenRevoke": "Revoca",
  "admin.tokenUsed": "già usato",
  "admin.tokenUnused": "mai usato",
  "admin.tokenNone": "Nessun token per questo account.",
  "admin.members": "Chi può vedere questa libreria",
  "admin.memberRole": "Ruolo",
  "admin.memberRemove": "Rimuovi",
  "admin.memberOwner": "proprietario, e lo resta",
  "admin.memberAdd": "Dai accesso a",
  "admin.memberChoose": "Scegli un account…",
  "admin.memberError": "Non è stato possibile modificare questo accesso.",
  "admin.roleAdmin": "amministratore",
  "admin.roleUser": "utente",
  "admin.roleOwner": "proprietario",
  "admin.roleManager": "gestore",
  "admin.roleListener": "ascoltatore",
  "admin.accountLibraries": {
    one: "{count} libreria",
    other: "{count} librerie",
  },
  "admin.manage": "Nome, cartella ed eliminazione",
  "admin.rename": "Nome",
  "admin.path": "Cartella sul server",
  "admin.followLinks":
    "Segui i link simbolici la cui destinazione resta dentro la libreria",
  "admin.watch":
    "Tieni d’occhio la cartella e scansiona ciò che cambia (da disattivare per una condivisione di rete modificata da altri computer)",
  "admin.save": "Salva",
  "admin.saved": "Salvato.",
  "admin.savedScan": "Salvato. Scansione {id} in coda.",
  "admin.saveError":
    "Non è stato possibile salvare la modifica. La cartella deve esistere sul server e non può essere un link simbolico.",
  "admin.saveBusy":
    "La libreria è in fase di scansione, oppure un’altra libreria usa già quella cartella. Riprova al termine della scansione.",
  "admin.delete": "Elimina questa libreria",
  "admin.deleteNote":
    "Rimuove la libreria dal catalogo per tutti i membri: i suoi brani, album e artisti, con le relative voci nelle playlist, i preferiti e gli ascolti. Nessun file sul disco viene toccato. Digita il nome della libreria per confermare.",
  "admin.deleteConfirm": "Il nome della libreria, per confermare",
  "admin.deleteButton": "Elimina per sempre",
  "admin.deleted":
    "{name} è stata eliminata: {tracks} brani sono usciti dal catalogo.",
  "admin.deleteError": "Non è stato possibile eliminare la libreria.",
  "scan.connecting": "In attesa della prima lettura…",
  "scan.queued": "In coda",
  "scan.running": "Scansione in corso",
  "scan.completed": "Completata",
  "scan.failed": "Non riuscita",
  "scan.progress": "Avanzamento della scansione",
  "scan.counted": "{processed} file su {total}",
  "scan.tally":
    "{added} aggiunti · {updated} aggiornati · {moved} spostati · {skipped} invariati · {errors} errori",
  "scan.lost":
    "Non è stato possibile aprire il flusso di avanzamento. La scansione in sé non ne risente.",
  "scan.leftOut": "{excluded} esclusi · {links} link non seguiti",
  "maintenance.sections": "Sezioni di manutenzione",
  "maintenance.none":
    "In questa libreria non c’è nulla di cui tu debba occuparti.",
  "missing.total": {
    one: "{count} file che le scansioni non trovano più",
    other: "{count} file che le scansioni non trovano più",
  },
  "missing.nothing": "Ogni file è dove il catalogo se lo aspetta.",
  "missing.notYours":
    "Solo i proprietari e i gestori di questa libreria, e gli amministratori, ne vedono i file mancanti.",
  "missing.note":
    "Questi brani restano nel catalogo, con i loro preferiti, le voci nelle playlist e gli ascolti, finché non vengono rimossi dal catalogo; se un file ricompare prima, conserva il suo brano. Per un certo periodo la rimozione si può annullare, poi il brano, insieme a tutto ciò che vi è collegato per ogni membro, sparisce per sempre.",
  "missing.since": "mancante dal {date}",
  "missing.cost":
    "{playlists} nelle playlist · {favorites} preferiti · {plays} ascolti",
  "missing.select": "Seleziona {title}",
  "missing.purgeSelected": "Rimuovi dal catalogo i selezionati ({count})",
  "missing.purgeAll": "Rimuovili tutti dal catalogo",
  "missing.confirm": "Conferma: rimuovi {count} dal catalogo",
  "missing.purged": {
    one: "{count} brano rimosso dal catalogo.",
    other: "{count} brani rimossi dal catalogo.",
  },
  "missing.scanning":
    "La libreria è in fase di scansione. Riprova al termine della scansione.",
  "missing.failed": "La rimozione non è riuscita. Riprova.",
  "missing.more": "Mostra altri",
  "missing.purgedUntil": {
    one: "{count} brano rimosso dal catalogo. Puoi ripristinarlo fino al {date}.",
    other:
      "{count} brani rimossi dal catalogo. Puoi ripristinarli fino al {date}.",
  },
  "missing.purgedTitle": {
    one: "Rimosso, ancora ripristinabile ({count})",
    other: "Rimossi, ancora ripristinabili ({count})",
  },
  "missing.purgedNote":
    "Di questi non è ancora andato perso nulla. Una volta ripristinato, un brano torna tra i file mancanti con tutto ciò che vi era collegato; se il file ricompare, il brano si ripristina da solo.",
  "missing.deletedAfter": "cancellato per sempre dopo il {date}",
  "missing.restore": "Ripristina",
  "missing.restoreOne": "Ripristina {title}",
  "missing.restoreAll": "Ripristinali tutti",
  "missing.restored": {
    one: "{count} brano ripristinato.",
    other: "{count} brani ripristinati.",
  },
  "missing.restoreFailed": "Il ripristino non è riuscito. Riprova.",
};
