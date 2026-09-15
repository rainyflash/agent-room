import { z } from 'zod';
import { validConversation } from '@/features/conversation/domain/conversation';

export const conversationSchema = z
  .object({
    attachmentName: z.string().min(1).max(240).optional(),
    text: z.string().min(1).max(8_000),
    mentions: z.array(z.string().min(4).max(255)).max(8),
  })
  .strict()
  .transform(({ attachmentName, ...chat }) => ({
    ...chat,
    ...(attachmentName === undefined ? {} : { attachmentName }),
  }))
  .refine(validConversation, '聊天文本或提及无效。');
