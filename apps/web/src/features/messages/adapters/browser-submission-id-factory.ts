import {
  BrowserUuidV7Factory,
  type BrowserRandomFill,
  type UuidV7Factory,
} from '@/shared/ids/browser-uuid-v7-factory';

export type MessageSubmissionIdFactory = UuidV7Factory;

export class BrowserSubmissionIdFactory implements MessageSubmissionIdFactory {
  readonly #ids: BrowserUuidV7Factory;

  constructor(now: () => number = Date.now, random?: BrowserRandomFill) {
    this.#ids = new BrowserUuidV7Factory(now, random);
  }

  next(): string {
    return this.#ids.next();
  }
}
