import { describe, expect, it, vi } from 'vitest';

import { MatrixMessageGateway } from './matrix-message-gateway';
import {
  matrixMessagePreviewEventType,
  matrixMessageRevisionEventType,
  type MatrixMessageRoomSnapshot,
  type MatrixMessageSource,
  type MatrixMessageSourceRead,
  type MatrixMessageTimelineEvent,
} from './matrix-message-source';

const NOW = Date.parse('2026-08-24T18:00:00.000Z');
const ROOM_ID = '!public:agent-room.test';
const MATRIX_USER_ID = '@build-agent:agent-room.test';
const AGENT_ID = '01990d9e-8400-7000-8000-000000000001';
const INSTANCE_ID = '01990d9e-8400-7000-8000-000000000002';
const MESSAGE_ID = '01990d9e-8400-7000-8000-000000000003';

describe('MatrixMessageGateway', () => {
  it('把 Matrix 生命周期错误映射为消息读取边界', () => {
    const unavailable = new MatrixMessageGateway(source({ kind: 'matrix-unavailable' }));
    const missing = new MatrixMessageGateway(source({ kind: 'room-not-joined' }));

    expect(unavailable.read(ROOM_ID)).toEqual({
      error: { code: 'messages.matrix_unavailable', retryable: true },
      ok: false,
    });
    expect(missing.read(ROOM_ID)).toEqual({
      error: { code: 'messages.room_not_joined', retryable: true },
      ok: false,
    });
  });

  it('只投影预览元数据，不读取正文，并按 Matrix 服务端顺序排列', () => {
    const room = snapshot([
      previewEvent({
        eventId: '$older',
        messageId: MESSAGE_ID,
        serverTimestamp: 100,
        title: '较早消息',
      }),
      previewEvent({
        eventId: '$newer',
        messageId: '01990d9e-8400-7000-8000-000000000004',
        serverTimestamp: 200,
        title: '较新消息',
      }),
    ]);
    const gateway = new MatrixMessageGateway(source({ kind: 'ready', room }), () => NOW);

    const result = gateway.read(ROOM_ID);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.observedAtUnixMs).toBe(NOW);
    expect(result.value.messages.map((message) => message.preview?.title)).toEqual([
      '较新消息',
      '较早消息',
    ]);
    expect(result.value.messages[0]?.content).toEqual({
      contentId: '01990d9e-8400-7000-8000-000000000006',
      digestSha256: 'a'.repeat(64),
      mediaType: 'text/markdown',
      sizeBytes: 1_024,
    });
    expect(Object.isFrozen(result.value)).toBe(true);
    expect(Object.isFrozen(result.value.messages)).toBe(true);
  });

  it('收敛先到达的编辑并让撤回成为终态', () => {
    const edit = revisionEvent({
      eventId: '$edit',
      kind: 'replace',
      targetMessageId: MESSAGE_ID,
      title: '已编辑标题',
    });
    const base = previewEvent({ eventId: '$base', messageId: MESSAGE_ID, title: '原始标题' });
    const redact = revisionEvent({
      eventId: '$redact',
      kind: 'redact',
      targetMessageId: MESSAGE_ID,
    });
    const lateEdit = revisionEvent({
      eventId: '$late-edit',
      kind: 'replace',
      targetMessageId: MESSAGE_ID,
      title: '不得复活',
    });
    const gateway = new MatrixMessageGateway(
      source({ kind: 'ready', room: snapshot([edit, base, redact, lateEdit]) }),
    );

    const result = gateway.read(ROOM_ID);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.messages).toEqual([
      expect.objectContaining({
        content: null,
        edited: true,
        lifecycle: 'redacted',
        messageId: MESSAGE_ID,
        preview: null,
      }),
    ]);
  });

  it('隔离伪造发送者、房间错配、媒体类型错配和重复 Matrix 事件', () => {
    const valid = previewEvent({ eventId: '$valid', messageId: MESSAGE_ID });
    const duplicate = { ...valid };
    const forged = { ...previewEvent({ eventId: '$forged' }), sender: '@attacker:agent-room.test' };
    const wrongRoom = previewEvent({ eventId: '$wrong-room' });
    wrongRoom.content = { ...messageContent(), roomId: '!other:agent-room.test' };
    const wrongMedia = previewEvent({ eventId: '$wrong-media' });
    wrongMedia.content = {
      ...messageContent(),
      content: { ...contentReference(), mediaType: 'text/plain' },
    };
    const gateway = new MatrixMessageGateway(
      source({
        kind: 'ready',
        room: snapshot([valid, duplicate, forged, wrongRoom, wrongMedia]),
      }),
    );

    const result = gateway.read(ROOM_ID);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.messages).toHaveLength(1);
    expect(result.value.messages[0]?.matrixEventId).toBe('$valid');
  });

  it('不允许其他发送者编辑原消息，并完整委托订阅生命周期', () => {
    const base = previewEvent({ eventId: '$base', messageId: MESSAGE_ID, title: '原始标题' });
    const forgedEdit = revisionEvent({
      eventId: '$edit',
      kind: 'replace',
      targetMessageId: MESSAGE_ID,
      title: '伪造编辑',
    });
    forgedEdit.sender = '@other:agent-room.test';
    forgedEdit.content = {
      ...forgedEdit.content,
      actor: {
        ...messageContent().actor,
        agent: { ...messageContent().actor.agent, matrixUserId: '@other:agent-room.test' },
      },
    };
    const unsubscribe = vi.fn();
    const subscribe = vi.fn(() => unsubscribe);
    const gateway = new MatrixMessageGateway({
      read: () => ({ kind: 'ready', room: snapshot([base, forgedEdit]) }),
      subscribe,
    });
    const listener = vi.fn();

    const result = gateway.read(ROOM_ID);
    const detach = gateway.subscribe(ROOM_ID, listener);
    detach();

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.messages[0]?.preview?.title).toBe('原始标题');
    }
    expect(subscribe).toHaveBeenCalledWith(ROOM_ID, listener);
    expect(unsubscribe).toHaveBeenCalledOnce();
  });
});

function source(read: MatrixMessageSourceRead): MatrixMessageSource {
  return { read: () => read, subscribe: () => noop };
}

function snapshot(
  timelineEvents: readonly MatrixMessageTimelineEvent[],
): MatrixMessageRoomSnapshot {
  return { roomId: ROOM_ID, timelineEvents };
}

type PreviewEventOptions = {
  readonly eventId: string;
  readonly messageId?: string;
  readonly serverTimestamp?: number;
  readonly title?: string;
};

function previewEvent(
  options: PreviewEventOptions,
): MatrixMessageTimelineEvent & { content: unknown } {
  return {
    content: messageContent({
      id: options.messageId ?? '01990d9e-8400-7000-8000-000000000020',
      ...(options.title === undefined ? {} : { title: options.title }),
    }),
    eventId: options.eventId,
    sender: MATRIX_USER_ID,
    serverTimestamp: options.serverTimestamp ?? 100,
    type: matrixMessagePreviewEventType,
  };
}

type RevisionEventOptions = {
  readonly eventId: string;
  readonly kind: 'redact' | 'replace';
  readonly targetMessageId: string;
  readonly title?: string;
};

function revisionEvent(
  options: RevisionEventOptions,
): MatrixMessageTimelineEvent & { content: Record<string, unknown>; sender: string | undefined } {
  const replacement = options.kind === 'replace';
  return {
    content: {
      actor: messageContent().actor,
      correlationId: '01990d9e-8400-7000-8000-000000000032',
      createdAt: '2026-08-24T17:01:00.000Z',
      eventType: matrixMessageRevisionEventType,
      id: '01990d9e-8400-7000-8000-000000000033',
      kind: options.kind,
      ...(replacement ? { content: contentReference() } : {}),
      ...(replacement ? { preview: previewContent(options.title ?? '替换后的消息') } : {}),
      roomId: ROOM_ID,
      schemaVersion: '1.0',
      signature: 'BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB',
      targetMessageId: options.targetMessageId,
    },
    eventId: options.eventId,
    sender: MATRIX_USER_ID,
    serverTimestamp: 150,
    type: matrixMessageRevisionEventType,
  };
}

function messageContent(options?: { readonly id?: string; readonly title?: string }) {
  return {
    actor: {
      agent: {
        agentId: AGENT_ID,
        avatarUrl: 'https://media.agent-room.test/build-agent.png',
        displayName: '构建助手',
        matrixUserId: MATRIX_USER_ID,
      },
      instanceId: INSTANCE_ID,
      provenance: 'human_confirmed_agent',
    },
    content: contentReference(),
    correlationId: '01990d9e-8400-7000-8000-000000000005',
    createdAt: '2026-08-24T17:00:00.000Z',
    eventType: matrixMessagePreviewEventType,
    id: options?.id ?? MESSAGE_ID,
    preview: previewContent(options?.title ?? '协议生成完成'),
    roomId: ROOM_ID,
    schemaVersion: '1.0',
    signature: 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
  } as const;
}

function previewContent(title: string) {
  return {
    contentType: 'text/markdown',
    language: 'zh-CN',
    riskFlags: ['untrusted_instructions'],
    sensitivity: 'normal',
    summary: '协议生成流水线已经完成，等待你决定是否读取正文。',
    title,
  } as const;
}

function contentReference() {
  return {
    contentId: '01990d9e-8400-7000-8000-000000000006',
    digestSha256: 'a'.repeat(64),
    fetchMode: 'on_demand',
    mediaType: 'text/markdown',
    sizeBytes: 1_024,
  } as const;
}

function noop(): void {
  return undefined;
}
