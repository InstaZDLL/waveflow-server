// The bytes the mocks serve: a silent WAV and a one-pixel image.



/**
 * Taking a station out of the bar leaves the queue's track there, paused
 * where it was, and the light theme keeps the live badge readable.
 */
/** A one-pixel PNG, for every cover the artwork route is asked for. */
export const PIXEL = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
  "base64",
);

/**
 * Seconds of 8 kHz, 8-bit mono silence as a WAV file: something the browser
 * decodes and plays for real, so `paused` and `currentTime` mean what they
 * say, at a size a route can answer in one piece.
 */
export function silentWav(seconds: number): Buffer {
  const rate = 8_000;
  const samples = rate * seconds;
  const header = Buffer.alloc(44);
  header.write("RIFF", 0);
  header.writeUInt32LE(36 + samples, 4);
  header.write("WAVE", 8);
  header.write("fmt ", 12);
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(1, 22);
  header.writeUInt32LE(rate, 24);
  header.writeUInt32LE(rate, 28);
  header.writeUInt16LE(1, 32);
  header.writeUInt16LE(8, 34);
  header.write("data", 36);
  header.writeUInt32LE(samples, 40);
  // Unsigned 8-bit samples sit at 128 when silent.
  return Buffer.concat([header, Buffer.alloc(samples, 128)]);
}
