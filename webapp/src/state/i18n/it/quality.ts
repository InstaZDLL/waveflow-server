// Audio quality labels and the quality analysis.
import type { TranslationValue } from "..";
import type { quality as en } from "../en/quality";

export const quality: Record<keyof typeof en, TranslationValue> = {
  "quality.label": "Qualità del file",
  "quality.unknown": "Sconosciuta",
  "quality.low": "Bassa",
  "quality.standard": "Standard",
  "quality.high": "Alta",
  "quality.lossless": "Lossless",
  "quality.hiRes": "Hi-Res",
  "quality.help":
    "Misurata dall’audio invece che letta dal contenitore: dove si fermano le alte frequenze e quanti bit usano i campioni. Un file lossless le cui alte frequenze si fermano intorno ai 16 kHz era un MP3 a 128 kbps prima di diventare un FLAC. «Nessun segno di una sorgente con perdita» non è una prova: un encoder che conserva tutta la banda non lascia nulla da trovare.",
  "quality.progress": "{measured} file lossless analizzati su {lossless}",
  "quality.disabled": "L’analisi è disattivata su questo server.",
  "quality.noLossless": "Nessun file lossless in questa libreria.",
  "quality.status.running":
    "Analisi in corso: {measured} su {lossless} completati, {inFlight} in lavorazione.",
  "quality.status.waitingForScan":
    "In attesa che la scansione finisca prima di analizzare.",
  "quality.status.queued": "In coda: l’analisi raggiungerà presto questi file.",
  "quality.status.idle": "Inattiva. Ultimo file analizzato il {when}.",
  "quality.status.idleNever":
    "Inattiva. Non è stato ancora analizzato nessun file.",
  "quality.filters": "Verdetti",
  "quality.all": "Tutti i file",
  "quality.format": "Formato",
  "quality.allFormats": "Tutti i formati",
  "quality.empty": "Nessun file con questo verdetto.",
  "quality.column.track": "Brano",
  "quality.column.container": "Cosa dice il file",
  "quality.column.highs": "Alte frequenze fino a",
  "quality.column.verdict": "Cosa dice l’audio",
  "quality.fullBand": "Banda completa",
  "quality.khz": "{value} kHz",
  "quality.db": "{value} dB",
  "quality.analyseAgain": "Analizza di nuovo",
  "quality.confirmAnalyse": "Conferma: analizza di nuovo tutta la libreria",
  "quality.requested": "La libreria verrà analizzata di nuovo in background.",
  "quality.requestFailed": "Non ha funzionato. Riprova.",
  "quality.section": "Qualità reale",
  "quality.verdictLabel": "Verdetto",
  "quality.highsStop": "Alte frequenze fino a",
  "quality.wall": "Calo in quel punto",
  "quality.bitsUsed": "Bit in uso, misurati",
  "quality.bitsOf": "{used} su {declared}",
  "quality.bitsNote":
    "Una misurazione dei campioni, non la prova di un vero hi-res: un file che usa tutti i bit che dichiara non è stato riempito artificialmente, ma questo non significa che sia stato registrato a quella profondità.",
  "quality.analysed": "Analizzato",
  "quality.chart": "Spettro medio: le alte frequenze si fermano a {where}.",
  "quality.chartFullBand":
    "Spettro medio: le alte frequenze proseguono senza alcun taglio.",
  "quality.showSpectrogram": "Mostra lo spettrogramma",
  "quality.drawing": "Disegno in corso…",
  "quality.spectrogramAlt": "Spettrogramma di {title}, frequenza nel tempo",
  "quality.spectrogramBusy":
    "Uno spettrogramma è già in fase di disegno per te. Riprova tra un attimo.",
  "quality.spectrogramFailed":
    "Non è stato possibile disegnare lo spettrogramma.",
  "quality.verdict.lossy_source": "Probabilmente da un file con perdita",
  "quality.verdict.upsampled": "Probabilmente sovracampionato",
  "quality.verdict.padded": "Profondità di bit gonfiata",
  "quality.verdict.unreadable": "Illeggibile",
  "quality.verdict.silent": "Silenzioso",
  "quality.verdict.pending": "In attesa di analisi",
  "quality.verdict.consistent": "Nessun segno di una sorgente con perdita",
  "quality.verdict.lossy": "Con perdita",
  "quality.verdict.not_measured": "Non misurato",
  "quality.explain.lossy_source":
    "Le alte frequenze si fermano a un taglio netto, dove taglia un encoder con perdita: questo file è stato probabilmente decodificato da un file MP3, AAC o Opus e salvato come lossless.",
  "quality.explain.upsampled":
    "La frequenza di campionamento supera i 48 kHz, ma le alte frequenze si fermano dove finisce un CD o una registrazione a 48 kHz: probabilmente è stato ricampionato da una di queste.",
  "quality.explain.padded":
    "Il contenitore dichiara più bit di quelli usati dai campioni: per esempio, una registrazione a 16 bit salvata a 24 bit.",
  "quality.explain.unreadable":
    "FFmpeg non è riuscito a decodificare questo file. Verrà riprovato quando il file cambierà.",
  "quality.explain.silent":
    "In questo file non c’è nulla di abbastanza forte da poter essere misurato.",
  "quality.explain.pending":
    "Questo file non è stato ancora analizzato. L’analisi viene eseguita in background.",
  "quality.explain.consistent":
    "Nulla nell’audio contraddice il contenitore. Non è una prova: un encoder che conserva tutta la banda non lascia nulla da trovare.",
  "quality.explain.lossy":
    "Un file con perdita dichiara ciò che è; la sua qualità dipende dall’encoder e dal bitrate. Non viene analizzato.",
  "quality.explain.not_measured": "Questo formato non viene analizzato.",
};
