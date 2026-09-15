import { validAttachmentName } from './conversation';
import type { Result } from '@/shared/result';

export const maximumAttachmentBytes = 20 * 1024 * 1024;
export type ConversationAttachment = {
  readonly name: string;
  readonly mediaType: string;
  readonly bytes: Uint8Array<ArrayBuffer>;
};
export type AttachmentFailure =
  'tooLarge' | 'empty' | 'invalid' | 'readFailed' | 'storageFailed' | 'cleanupFailed';
export type AttachmentReference = { readonly key: string; readonly name: string };
export type AttachmentState =
  | { readonly kind: 'none' }
  | { readonly kind: 'loading'; readonly reference: AttachmentReference }
  | { readonly kind: 'missing'; readonly reference: AttachmentReference }
  | {
      readonly kind: 'ready';
      readonly reference: AttachmentReference;
      readonly file: ConversationAttachment;
    };
export type ConversationAttachmentStorage = {
  read(key: string): Promise<Result<ConversationAttachment | null, AttachmentFailure>>;
  write(key: string, file: ConversationAttachment): Promise<Result<void, AttachmentFailure>>;
  remove(key: string): Promise<Result<void, AttachmentFailure>>;
};

export function attachmentIssue(file: ConversationAttachment): AttachmentFailure | null {
  if (file.bytes.byteLength === 0) return 'empty';
  if (file.bytes.byteLength > maximumAttachmentBytes) return 'tooLarge';
  if (
    !validAttachmentName(file.name) ||
    file.mediaType.length > 128 ||
    !/^[a-z0-9!#$&^_.+-]+\/[a-z0-9!#$&^_.+-]+$/u.test(file.mediaType)
  )
    return 'invalid';
  return null;
}
export function imageAttachment(mediaType: string): boolean {
  return ['image/png', 'image/jpeg', 'image/gif', 'image/webp', 'image/avif'].includes(mediaType);
}
export function attachmentSize(bytes: number): string {
  return bytes >= 1024 * 1024
    ? `${(bytes / (1024 * 1024)).toFixed(1)} MB`
    : `${String(Math.max(1, Math.ceil(bytes / 1024)))} KB`;
}
