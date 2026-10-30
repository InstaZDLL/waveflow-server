// Audio quality labels and the quality analysis.
import type { TranslationValue } from "..";
import type { quality as en } from "../en/quality";

export const quality: Record<keyof typeof en, TranslationValue> = {
  "quality.label": "Calidad del archivo",
  "quality.unknown": "Desconocida",
  "quality.low": "Baja",
  "quality.standard": "Estándar",
  "quality.high": "Alta",
  "quality.lossless": "Sin pérdida",
  "quality.hiRes": "Alta resolución",
  "quality.help":
    "Se mide en el audio en lugar de leerse del contenedor: dónde se cortan los agudos y cuántos bits usan las muestras. Un archivo sin pérdida cuyos agudos se cortan hacia los 16 kHz fue un MP3 de 128 kbps antes de ser un FLAC. “Sin rastro de una fuente con pérdida” no es una prueba: un codificador que conserva toda la banda no deja nada que encontrar.",
  "quality.progress":
    "{measured} de {lossless} archivos sin pérdida analizados",
  "quality.disabled": "El análisis está desactivado en este servidor.",
  "quality.noLossless": "No hay ningún archivo sin pérdida en esta biblioteca.",
  "quality.status.running":
    "Analizando: {measured} de {lossless} terminados, {inFlight} en curso.",
  "quality.status.waitingForScan":
    "Esperando a que termine el escaneo para analizar.",
  "quality.status.queued":
    "En cola: el análisis llegará pronto a estos archivos.",
  "quality.status.idle": "En reposo. Último archivo analizado el {when}.",
  "quality.status.idleNever":
    "En reposo. Todavía no se ha analizado ningún archivo.",
  "quality.filters": "Veredictos",
  "quality.all": "Todos los archivos",
  "quality.format": "Formato",
  "quality.allFormats": "Todos los formatos",
  "quality.empty": "Ningún archivo tiene este veredicto.",
  "quality.column.track": "Pista",
  "quality.column.container": "Lo que dice el archivo",
  "quality.column.highs": "Los agudos se cortan en",
  "quality.column.verdict": "Lo que dice el audio",
  "quality.fullBand": "Banda completa",
  "quality.khz": "{value} kHz",
  "quality.db": "{value} dB",
  "quality.analyseAgain": "Volver a analizar",
  "quality.confirmAnalyse": "Confirmar: volver a analizar toda la biblioteca",
  "quality.requested": "La biblioteca se volverá a analizar en segundo plano.",
  "quality.requestFailed": "No funcionó. Vuelve a intentarlo.",
  "quality.section": "Calidad real",
  "quality.verdictLabel": "Veredicto",
  "quality.highsStop": "Los agudos se cortan en",
  "quality.wall": "Caída en ese punto",
  "quality.bitsUsed": "Bits en uso, medidos",
  "quality.bitsOf": "{used} de {declared}",
  "quality.bitsNote":
    "Una medición de las muestras, no una prueba de alta resolución auténtica: un archivo que usa todos los bits que declara no fue rellenado, pero eso no significa que se grabara con esa profundidad.",
  "quality.analysed": "Analizado",
  "quality.chart": "Espectro promedio: los agudos se cortan en {where}.",
  "quality.chartFullBand":
    "Espectro promedio: los agudos continúan sin ningún corte.",
  "quality.showSpectrogram": "Mostrar el espectrograma",
  "quality.drawing": "Dibujando…",
  "quality.spectrogramAlt":
    "Espectrograma de {title}, frecuencia frente a tiempo",
  "quality.spectrogramBusy":
    "Ya se está dibujando un espectrograma para ti. Vuelve a intentarlo en un momento.",
  "quality.spectrogramFailed": "No se pudo dibujar el espectrograma.",
  "quality.verdict.lossy_source": "Probablemente de un archivo con pérdida",
  "quality.verdict.upsampled": "Probablemente sobremuestreado",
  "quality.verdict.padded": "Profundidad de bits rellenada",
  "quality.verdict.unreadable": "No se pudo leer",
  "quality.verdict.silent": "Silencio",
  "quality.verdict.pending": "Pendiente de análisis",
  "quality.verdict.consistent": "Sin rastro de una fuente con pérdida",
  "quality.verdict.lossy": "Con pérdida",
  "quality.verdict.not_measured": "Sin medir",
  "quality.explain.lossy_source":
    "Los agudos se cortan en seco donde corta un codificador con pérdida: este archivo probablemente se decodificó de un archivo MP3, AAC u Opus y se guardó como sin pérdida.",
  "quality.explain.upsampled":
    "La frecuencia de muestreo supera los 48 kHz, pero los agudos se cortan donde termina un CD o una grabación de 48 kHz: probablemente se remuestreó a partir de uno de ellos.",
  "quality.explain.padded":
    "El contenedor declara más bits de los que usan las muestras: por ejemplo, una grabación de 16 bits guardada en 24 bits.",
  "quality.explain.unreadable":
    "FFmpeg no pudo decodificar este archivo. Se volverá a intentar cuando el archivo cambie.",
  "quality.explain.silent":
    "No hay nada en este archivo con volumen suficiente para medirlo.",
  "quality.explain.pending":
    "Este archivo todavía no se ha analizado. El análisis se hace en segundo plano.",
  "quality.explain.consistent":
    "Nada en el audio contradice al contenedor. No es una prueba: un codificador que conserva toda la banda no deja nada que encontrar.",
  "quality.explain.lossy":
    "Un archivo con pérdida dice lo que es; su calidad depende de su codificador y su tasa de bits. No se analiza.",
  "quality.explain.not_measured": "Este formato no se analiza.",
};
