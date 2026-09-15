import type { RoomMessageSignal } from '@/features/messages/domain/message';

export function conversationFixture(
  id: string,
  options: Partial<RoomMessageSignal> = {},
): RoomMessageSignal {
  return {
    actor: {
      kind: 'human',
      principalId: '0198b601-77a1-7bb8-83eb-a8fe68c97e45',
      displayName: 'Alex',
      matrixUserId: '@alex:example.org',
      provenance: 'human',
    },
    messageId: id,
    matrixEventId: `$${id}`,
    roomId: '!room:example.org',
    serverTimestamp: new Date('2026-09-14T12:00:00').getTime(),
    lifecycle: 'active',
    edited: false,
    endToEndEncrypted: false,
    signatureStatus: 'matrix_sender_matched',
    content: null,
    preview: {
      title: 'Conversation',
      summary: 'Hello',
      conversation: { text: 'Hello', mentions: [] },
      contentType: 'text/plain',
      riskFlags: [],
      sensitivity: 'normal',
    },
    ...options,
  };
}
