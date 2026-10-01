import type { MessageRelation, RoomMessageSignal } from '@/features/messages/domain/message';

export type ConversationMessage = {
  readonly attachmentName?: string;
  readonly text: string;
  readonly mentions: readonly string[];
};

export type ConversationParticipant = {
  readonly displayName: string;
  readonly matrixUserId: string;
};

export const maximumChatCharacters = 4_000;
/** 一条消息最多点名几个人，和服务器、Bridge 一样（specs/agent-reading/mentions.md）。 */
export const maximumMentions = 200;
/** 点名的 ID 加起来最多这么多字节，免得撑破 Matrix 事件。 */
export const maximumMentionBytes = 12 * 1_024;
const maximumMatrixIdBytes = 255;

export function mentionBytes(mentions: readonly string[]): number {
  const encoder = new TextEncoder();
  return mentions.reduce((total, id) => total + encoder.encode(id).length, 0);
}

/** 点满了：个数到了上限，或者剩下的字节放不下一个最长的 ID。 */
export function mentionsFull(mentions: readonly string[]): boolean {
  return (
    mentions.length >= maximumMentions ||
    mentionBytes(mentions) + maximumMatrixIdBytes > maximumMentionBytes
  );
}

export function validConversation(chat: ConversationMessage): boolean {
  return (
    (chat.attachmentName === undefined || validAttachmentName(chat.attachmentName)) &&
    chat.text.trim().length > 0 &&
    Array.from(chat.text).length <= maximumChatCharacters &&
    !Array.from(chat.text).some((character) => {
      const code = character.codePointAt(0) ?? 0;
      return (code < 32 && code !== 9 && code !== 10) || (code >= 127 && code <= 159);
    }) &&
    chat.mentions.length <= maximumMentions &&
    mentionBytes(chat.mentions) <= maximumMentionBytes &&
    new Set(chat.mentions).size === chat.mentions.length &&
    chat.mentions.every(
      (id) =>
        new TextEncoder().encode(id).length <= maximumMatrixIdBytes &&
        /^@[^\s:]+:[^\s]+$/u.test(id) &&
        !Array.from(id).some((character) => {
          const code = character.codePointAt(0) ?? 0;
          return code < 32 || (code >= 127 && code <= 159);
        }),
    )
  );
}

export function validAttachmentName(name: string): boolean {
  return (
    name.trim().length > 0 &&
    Array.from(name).length <= 120 &&
    !/[\p{Cc}/\\]/u.test(name) &&
    name !== '.' &&
    name !== '..'
  );
}

export function conversationMessages(
  messages: readonly RoomMessageSignal[],
): readonly RoomMessageSignal[] {
  return messages
    .filter(
      (message) => message.lifecycle === 'active' && message.preview?.conversation !== undefined,
    )
    .toSorted(
      (left, right) =>
        left.serverTimestamp - right.serverTimestamp ||
        left.matrixEventId.localeCompare(right.matrixEventId),
    );
}

export function replyRelation(message: Pick<RoomMessageSignal, 'messageId'>): MessageRelation {
  return Object.freeze({ kind: 'reply', targetMessageId: message.messageId });
}
