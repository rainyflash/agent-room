import type { MessageContentReference } from '@/features/messages/domain/message';
import type { Result } from '@/shared/result';

export type ContentReadTicket = {
  readonly expiresAtUnixMs: number;
  readonly ticket: string;
};

export type DownloadedContent = {
  readonly bytes: Uint8Array;
  readonly contentDigest: string | null;
  readonly contentLength: string | null;
  readonly mediaType: string | null;
};

export type VerifiedContent = {
  readonly bytes: Uint8Array;
  readonly digestSha256: string;
  readonly mediaType: string;
  readonly mode: 'download' | 'text';
  readonly text?: string;
};

export type ContentFailureCode =
  | 'content.offline'
  | 'content.timeout'
  | 'content.ticket_rejected'
  | 'content.download_rejected'
  | 'content.invalid_response'
  | 'content.length_mismatch'
  | 'content.digest_mismatch'
  | 'content.media_type_mismatch'
  | 'content.invalid_text';

export type ContentFailure = {
  readonly code: ContentFailureCode;
  readonly correlationId?: string;
  readonly retryable: boolean;
};

export type ContentGateway = {
  download(contentId: string, ticket: string): Promise<Result<DownloadedContent, ContentFailure>>;
  issueReadTicket(contentId: string): Promise<Result<ContentReadTicket, ContentFailure>>;
};

export type ContentVerifier = {
  verify(
    downloaded: DownloadedContent,
    expected: MessageContentReference,
  ): Promise<Result<VerifiedContent, ContentFailure>>;
};
