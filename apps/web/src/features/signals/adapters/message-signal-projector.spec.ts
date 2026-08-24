import { describe, expect, it } from 'vitest';

import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { projectMessageSignals } from '@/features/signals/adapters/message-signal-projector';

describe('projectMessageSignals', () => {
  it('只把房间消息事实投影成信号，不补造正文或来源', () => {
    const message = roomMessage();

    expect(projectMessageSignals([message], 'room')).toEqual([
      {
        action: { kind: 'open_message', messageId: message.messageId },
        actor: message.actor,
        edited: false,
        kind: 'room_message',
        lifecycle: 'active',
        occurredAtUnixMs: 1_700_000_000_000,
        riskFlags: ['untrusted_instructions'],
        signalId: `message:${message.messageId}`,
        summary: 'Waiting for content approval',
        title: 'Protocol generation complete',
      },
    ]);
  });

  it('直接会话由房间情境决定，撤回消息不会残留旧预览', () => {
    const message = roomMessage();
    const [projected] = projectMessageSignals(
      [{ ...message, content: null, lifecycle: 'redacted', preview: null }],
      'direct',
    );

    expect(projected).toMatchObject({
      kind: 'direct_message',
      lifecycle: 'redacted',
      summary: null,
      title: null,
    });
  });
});

function roomMessage(): RoomMessageSignal {
  return {
    actor: {
      agentId: '01990d9e-8400-7000-8000-000000000001',
      displayName: 'Build Agent',
      instanceId: '01990d9e-8400-7000-8000-000000000002',
      matrixUserId: '@build-agent:agent-room.test',
      provenance: 'human_confirmed_agent',
    },
    content: {
      contentId: '01990d9e-8400-7000-8000-000000000006',
      digestSha256: 'ab'.repeat(32),
      mediaType: 'text/markdown',
      sizeBytes: 128,
    },
    edited: false,
    lifecycle: 'active',
    matrixEventId: '$message',
    messageId: '01990d9e-8400-7000-8000-000000000003',
    preview: {
      contentType: 'text/markdown',
      riskFlags: ['untrusted_instructions'],
      sensitivity: 'normal',
      summary: 'Waiting for content approval',
      title: 'Protocol generation complete',
    },
    roomId: '!public:agent-room.test',
    serverTimestamp: 1_700_000_000_000,
  };
}
