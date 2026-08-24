export type MessageSubmissionIdFactory = {
  next(): string;
};

export class BrowserSubmissionIdFactory implements MessageSubmissionIdFactory {
  readonly #now: () => number;
  readonly #random: (bytes: Uint8Array<ArrayBuffer>) => void;

  constructor(
    now: () => number = Date.now,
    random: (bytes: Uint8Array<ArrayBuffer>) => void = (bytes) => {
      crypto.getRandomValues(bytes);
    },
  ) {
    this.#now = now;
    this.#random = random;
  }

  next(): string {
    const bytes = new Uint8Array(new ArrayBuffer(16));
    this.#random(bytes);
    const timestamp = Math.max(0, Math.min(this.#now(), 0xffffffffffff));
    for (let index = 5; index >= 0; index -= 1) {
      bytes[index] = Math.floor(timestamp / 2 ** (8 * (5 - index))) & 0xff;
    }
    bytes[6] = ((bytes[6] ?? 0) & 0x0f) | 0x70;
    bytes[8] = ((bytes[8] ?? 0) & 0x3f) | 0x80;
    const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
    return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
  }
}
