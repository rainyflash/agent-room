import type {
  MessageBodyPreparer,
  MessagePublicationFailure,
  PreparedMessageBody,
} from '@/features/messages/domain/publication';
import { err, ok, type Result } from '@/shared/result';

export type BrowserMessageBodyPreparerOptions = {
  readonly digest?: (bytes: Uint8Array<ArrayBuffer>) => Promise<ArrayBuffer>;
};

export class BrowserMessageBodyPreparer implements MessageBodyPreparer {
  readonly #digest: (bytes: Uint8Array<ArrayBuffer>) => Promise<ArrayBuffer>;

  constructor({ digest = browserSha256 }: BrowserMessageBodyPreparerOptions = {}) {
    this.#digest = digest;
  }

  async prepare(body: string): Promise<Result<PreparedMessageBody, MessagePublicationFailure>> {
    const encoded = new TextEncoder().encode(body);
    const bytes = new Uint8Array(encoded.byteLength);
    bytes.set(encoded);
    try {
      const digest = new Uint8Array(await this.#digest(bytes));
      return digest.byteLength === 32
        ? ok(Object.freeze({ bytes, digestSha256: toHex(digest) }))
        : err(failure());
    } catch {
      return err(failure());
    }
  }
}

function browserSha256(bytes: Uint8Array<ArrayBuffer>): Promise<ArrayBuffer> {
  return globalThis.crypto.subtle.digest('SHA-256', bytes);
}

function toHex(bytes: Uint8Array): string {
  return [...bytes].map((byte) => byte.toString(16).padStart(2, '0')).join('');
}

function failure(): MessagePublicationFailure {
  return Object.freeze({ code: 'publication.unexpected_failure', retryable: false });
}
