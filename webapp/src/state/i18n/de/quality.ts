// Audio quality labels and the quality analysis.
import type { TranslationValue } from "..";
import type { quality as en } from "../en/quality";

export const quality: Record<keyof typeof en, TranslationValue> = {
  "quality.label": "Dateiqualität",
  "quality.unknown": "Unbekannt",
  "quality.low": "Niedrig",
  "quality.standard": "Standard",
  "quality.high": "Hoch",
  "quality.lossless": "Verlustfrei",
  "quality.hiRes": "Hi-Res",
  "quality.help":
    "Aus dem Audio gemessen statt aus dem Container gelesen: wo die Höhen enden und wie viele Bits die Samples nutzen. Eine verlustfreie Datei, deren Höhen bei etwa 16 kHz enden, war ein MP3 mit 128 kbit/s, bevor sie ein FLAC wurde. „Kein Hinweis auf eine verlustbehaftete Quelle“ ist kein Beweis: Ein Encoder, der das volle Frequenzband erhält, hinterlässt keine Spuren.",
  "quality.progress":
    "{measured} von {lossless} verlustfreien Dateien analysiert",
  "quality.disabled": "Die Analyse ist auf diesem Server abgeschaltet.",
  "quality.noLossless": "Keine verlustfreie Datei in dieser Bibliothek.",
  "quality.status.running":
    "Analyse läuft: {measured} von {lossless} fertig, {inFlight} in Arbeit.",
  "quality.status.waitingForScan":
    "Die Analyse wartet, bis der Scan abgeschlossen ist.",
  "quality.status.queued":
    "Eingereiht: Die Analyse erreicht diese Dateien in Kürze.",
  "quality.status.idle": "Ruht. Letzte Datei analysiert {when}.",
  "quality.status.idleNever": "Ruht. Noch wurde keine Datei analysiert.",
  "quality.filters": "Ergebnisse",
  "quality.all": "Alle Dateien",
  "quality.format": "Format",
  "quality.allFormats": "Alle Formate",
  "quality.empty": "Keine Datei mit diesem Ergebnis.",
  "quality.column.track": "Titel",
  "quality.column.container": "Laut Datei",
  "quality.column.highs": "Höhen enden bei",
  "quality.column.verdict": "Laut Audio",
  "quality.fullBand": "Volles Band",
  "quality.khz": "{value} kHz",
  "quality.db": "{value} dB",
  "quality.analyseAgain": "Erneut analysieren",
  "quality.confirmAnalyse": "Bestätigen: ganze Bibliothek erneut analysieren",
  "quality.requested": "Die Bibliothek wird im Hintergrund erneut analysiert.",
  "quality.requestFailed": "Das hat nicht geklappt. Versuch es noch einmal.",
  "quality.section": "Echte Qualität",
  "quality.verdictLabel": "Ergebnis",
  "quality.highsStop": "Höhen enden bei",
  "quality.wall": "Abfall an dieser Stelle",
  "quality.bitsUsed": "Genutzte Bits, gemessen",
  "quality.bitsOf": "{used} von {declared}",
  "quality.bitsNote":
    "Eine Messung der Samples, kein Beweis für echtes Hi-Res: Eine Datei, die jedes angegebene Bit nutzt, wurde nicht aufgefüllt – das heißt aber nicht, dass sie in dieser Tiefe aufgenommen wurde.",
  "quality.analysed": "Analysiert",
  "quality.chart": "Gemitteltes Spektrum: Die Höhen enden bei {where}.",
  "quality.chartFullBand":
    "Gemitteltes Spektrum: Die Höhen reichen ohne Abbruchkante durch.",
  "quality.showSpectrogram": "Spektrogramm anzeigen",
  "quality.drawing": "Wird gezeichnet…",
  "quality.spectrogramAlt": "Spektrogramm von {title}, Frequenz über Zeit",
  "quality.spectrogramBusy":
    "Für dich wird bereits ein Spektrogramm gezeichnet. Versuch es gleich noch einmal.",
  "quality.spectrogramFailed":
    "Das Spektrogramm konnte nicht gezeichnet werden.",
  "quality.verdict.lossy_source":
    "Wahrscheinlich aus einer verlustbehafteten Datei",
  "quality.verdict.upsampled": "Wahrscheinlich hochgerechnet",
  "quality.verdict.padded": "Aufgefüllte Bittiefe",
  "quality.verdict.unreadable": "Nicht lesbar",
  "quality.verdict.silent": "Stille",
  "quality.verdict.pending": "Wartet auf Analyse",
  "quality.verdict.consistent": "Kein Hinweis auf eine verlustbehaftete Quelle",
  "quality.verdict.lossy": "Verlustbehaftet",
  "quality.verdict.not_measured": "Nicht gemessen",
  "quality.explain.lossy_source":
    "Die Höhen enden an einer Kante, an der ein verlustbehafteter Encoder abschneidet: Diese Datei wurde wahrscheinlich aus einer MP3-, AAC- oder Opus-Datei dekodiert und verlustfrei gespeichert.",
  "quality.explain.upsampled":
    "Die Abtastrate liegt über 48 kHz, aber die Höhen enden dort, wo eine CD oder eine 48-kHz-Aufnahme aufhört: Die Datei wurde wahrscheinlich aus einer solchen Aufnahme hochgerechnet.",
  "quality.explain.padded":
    "Der Container gibt mehr Bits an, als die Samples nutzen: zum Beispiel eine 16-Bit-Aufnahme, gespeichert mit 24 Bit.",
  "quality.explain.unreadable":
    "FFmpeg konnte diese Datei nicht dekodieren. Sie wird erneut versucht, wenn sich die Datei ändert.",
  "quality.explain.silent":
    "Nichts in dieser Datei ist laut genug, um es zu messen.",
  "quality.explain.pending":
    "Diese Datei wurde noch nicht analysiert. Die Analyse läuft im Hintergrund.",
  "quality.explain.consistent":
    "Nichts im Audio widerspricht dem Container. Das ist kein Beweis: Ein Encoder, der das volle Frequenzband erhält, hinterlässt keine Spuren.",
  "quality.explain.lossy":
    "Eine verlustbehaftete Datei sagt, was sie ist; ihre Qualität bestimmen Encoder und Bitrate. Sie wird nicht analysiert.",
  "quality.explain.not_measured": "Dieses Format wird nicht analysiert.",
};
