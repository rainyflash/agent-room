import type { MessageRelation, RoomMessageSignal } from '@/features/messages/domain/message';

export type ConversationMessage = {
  readonly text: string;
  readonly mentions: readonly string[];
};

export type ConversationParticipant = {
  readonly displayName: string;
  readonly matrixUserId: string;
};

export const maximumChatCharacters = 4_000;
export const maximumMentions = 8;

export function validConversation(chat: ConversationMessage): boolean {
  return (
    chat.text.trim().length > 0 &&
    Array.from(chat.text).length <= maximumChatCharacters &&
    !Array.from(chat.text).some((character) => {
      const code = character.codePointAt(0) ?? 0;
      return (code < 32 && code !== 9 && code !== 10) || (code >= 127 && code <= 159);
    }) &&
    chat.mentions.length <= maximumMentions &&
    new Set(chat.mentions).size === chat.mentions.length &&
    chat.mentions.every(
      (id) =>
        new TextEncoder().encode(id).length <= 255 &&
        /^@[^\s:]+:[^\s]+$/u.test(id) &&
        !Array.from(id).some((character) => {
          const code = character.codePointAt(0) ?? 0;
          return code < 32 || (code >= 127 && code <= 159);
        }),
    )
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

export function replyRelation(message: RoomMessageSignal): MessageRelation {
  return Object.freeze({ kind: 'reply', targetMessageId: message.messageId });
}
