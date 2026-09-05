import { decryptContent } from './browser-content-cipher';
import type { MessageContentReference } from '@/features/messages/domain/message';
import type {
  ContentFailure,
  ContentVerifier,
  DownloadedContent,
  VerifiedContent,
} from '@/features/messages/domain/content';
import { err, ok, type Result } from '@/shared/result';

const textMediaTypes = new Set(['application/json', 'text/markdown', 'text/plain']);
const contentDigestPattern = /^sha-256=:([A-Za-z0-9+/]+={0,2}):$/u;

export type BrowserContentVerifierOptions = {
  readonly digest?: (bytes: Uint8Array) => Promise<ArrayBuffer>;
};

export class BrowserContentVerifier implements ContentVerifier {
  readonly #digest: (bytes: Uint8Array) => Promise<ArrayBuffer>;

  constructor({
    digest = async (bytes) => {
      const ownedBytes = Uint8Array.from(bytes);
      return await globalThis.crypto.subtle.digest('SHA-256', ownedBytes.buffer);
    },
  }: BrowserContentVerifierOptions = {}) {
    this.#digest = digest;
  }

  async verify(
    downloaded: DownloadedContent,
    expected: MessageContentReference,
    roomId?: string,
  ): Promise<Result<VerifiedContent, ContentFailure>> {
    if (
      downloaded.bytes.byteLength !== expected.sizeBytes ||
      parseContentLength(downloaded.contentLength) !== expected.sizeBytes
    ) {
      return err(failure('content.length_mismatch'));
    }
    const mediaType = normalizeMediaType(downloaded.mediaType);
    if (mediaType === null || mediaType !== expected.mediaType) {
      return err(failure('content.media_type_mismatch'));
    }
    const headerDigest = parseContentDigest(downloaded.contentDigest);
    if (headerDigest === null || headerDigest !== expected.digestSha256) {
      return err(failure('content.digest_mismatch'));
    }

    let actualDigest: string;
    try {
      actualDigest = toHex(new Uint8Array(await this.#digest(downloaded.bytes)));
    } catch {
      return err(failure('content.digest_mismatch'));
    }
    if (actualDigest !== expected.digestSha256) {
      return err(failure('content.digest_mismatch'));
    }

    let bytes = downloaded.bytes;
    if (expected.encryption !== undefined) {
      if (roomId === undefined) return err(failure('content.invalid_response'));
      try {
        bytes = await decryptContent(bytes, expected.encryption, roomId, mediaType);
      } catch {
        return err(failure('content.digest_mismatch'));
      }
    }
    if (!textMediaTypes.has(mediaType)) {
      return ok(
        Object.freeze({
          bytes,
          digestSha256: actualDigest,
          mediaType,
          mode: 'download',
        }),
      );
    }
    try {
      const text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
      return ok(
        Object.freeze({
          bytes,
          digestSha256: actualDigest,
          mediaType,
          mode: 'text',
          text,
        }),
      );
    } catch {
      return err(failure('content.invalid_text'));
    }
  }
}

function parseContentLength(value: string | null): number | null {
  if (value === null || !/^(?:0|[1-9][0-9]*)$/u.test(value)) {
    return null;
  }
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) ? parsed : null;
}

function normalizeMediaType(value: string | null): string | null {
  if (value === null) {
    return null;
  }
  const mediaType = value.split(';', 1)[0]?.trim().toLowerCase();
  return mediaType === undefined || mediaType.length === 0 ? null : mediaType;
}

function parseContentDigest(value: string | null): string | null {
  if (value === null) {
    return null;
  }
  const match = contentDigestPattern.exec(value);
  const encoded = match?.[1];
  if (encoded === undefined) {
    return null;
  }
  try {
    const binary = globalThis.atob(encoded);
    return toHex(Uint8Array.from(binary, (character) => character.charCodeAt(0)));
  } catch {
    return null;
  }
}

function toHex(bytes: Uint8Array): string {
  return [...bytes].map((byte) => byte.toString(16).padStart(2, '0')).join('');
}

function failure(code: ContentFailure['code']): ContentFailure {
  return Object.freeze({ code, retryable: false });
}
