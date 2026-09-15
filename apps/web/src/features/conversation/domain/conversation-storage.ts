import { z } from 'zod';
import { err, ok, type Result } from '@/shared/result';

const replySchema = z.object({
  messageId: z.string().min(1).max(255),
  actor: z.object({ displayName: z.string().max(256), matrixUserId: z.string().max(255) }),
});
const savedConversationSchema = z.object({
  attachment: z.object({ key: z.string().max(255), name: z.string().max(240) }).optional(),
  version: z.literal(1),
  text: z.string().max(8000),
  mentions: z.array(z.string().max(255)).max(8),
  reply: replySchema.nullable(),
  pendingSubmissionId: z.uuid().nullable(),
});
export type ConversationReply = z.infer<typeof replySchema>;
export type SavedConversation = z.infer<typeof savedConversationSchema>;
export type ConversationStorage = {
  read(roomId: string): Result<SavedConversation | null, 'unavailable'>;
  write(roomId: string, draft: SavedConversation): Result<void, 'unavailable'>;
};

/** Account and room form the storage boundary, including interrupted submissions. */
export class BrowserConversationStorage implements ConversationStorage {
  constructor(
    private readonly storage: Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>,
    private readonly account: string,
  ) {}
  private key(roomId: string) {
    return `agent-room.conversation.v1:${encodeURIComponent(this.account)}:${encodeURIComponent(roomId)}`;
  }
  read(roomId: string): Result<SavedConversation | null, 'unavailable'> {
    try {
      const raw = this.storage.getItem(this.key(roomId));
      if (raw === null) return ok(null);
      const parsed = savedConversationSchema.safeParse(JSON.parse(raw));
      return parsed.success ? ok(parsed.data) : err('unavailable');
    } catch {
      return err('unavailable');
    }
  }
  write(roomId: string, draft: SavedConversation): Result<void, 'unavailable'> {
    try {
      if (
        draft.text === '' &&
        draft.mentions.length === 0 &&
        draft.reply === null &&
        draft.pendingSubmissionId === null &&
        draft.attachment === undefined
      )
        this.storage.removeItem(this.key(roomId));
      else this.storage.setItem(this.key(roomId), JSON.stringify(draft));
      return ok(undefined);
    } catch {
      return err('unavailable');
    }
  }
}
