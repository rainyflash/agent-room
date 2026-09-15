import { z } from 'zod';
import { BrowserBinaryStore } from '@/shared/storage/browser-binary-store';
import { err, ok } from '@/shared/result';
import {
  attachmentIssue,
  type ConversationAttachment,
  type ConversationAttachmentStorage,
} from '../domain/conversation-attachment';

const schema = z.object({
  name: z.string(),
  mediaType: z.string(),
  bytes: z.instanceof(Uint8Array),
});
export class BrowserAttachmentStorage implements ConversationAttachmentStorage {
  private readonly store: BrowserBinaryStore;
  constructor(account: string) {
    this.store = new BrowserBinaryStore(`conversation:${encodeURIComponent(account)}`);
  }
  async read(key: string) {
    try {
      const raw = await this.store.read(key);
      if (raw === undefined) return ok(null);
      const parsed = schema.safeParse(raw);
      if (!parsed.success) return err('storageFailed' as const);
      const file = { ...parsed.data, bytes: Uint8Array.from(parsed.data.bytes) };
      return attachmentIssue(file) === null ? ok(file) : err('storageFailed' as const);
    } catch {
      return err('storageFailed' as const);
    }
  }
  async write(key: string, file: ConversationAttachment) {
    try {
      await this.store.write(key, file);
      return ok(undefined);
    } catch {
      return err('storageFailed' as const);
    }
  }
  async remove(key: string) {
    try {
      await this.store.remove(key);
      return ok(undefined);
    } catch {
      return err('cleanupFailed' as const);
    }
  }
}
