import { describe, expect, it } from 'vitest';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import type { ReceiverView } from '@/features/desktop/domain/reception';
import { inviteReplyProgress } from '@/features/desktop/domain/invite-reply';
import { receiverAgentId, receiverStatus } from '@/features/desktop/domain/reception-status';
import { conversationDeliveries, messageDelivery } from './message-delivery';

const request: RoomMessageSignal = {
  roomId: '!room:test',
  messageId: 'question',
  matrixEventId: '$question',
  serverTimestamp: 100,
  actor: {
    kind: 'human',
    displayName: 'Owner',
    matrixUserId: '@owner:test',
    principalId: 'owner',
    provenance: 'human',
  },
  content: null,
  preview: null,
  edited: false,
  endToEndEncrypted: false,
  lifecycle: 'active',
  signatureStatus: 'matrix_sender_matched',
};
const reply: RoomMessageSignal = {
  ...request,
  messageId: 'answer',
  matrixEventId: '$answer',
  serverTimestamp: 110,
  actor: {
    kind: 'agent',
    displayName: 'Agent',
    agentId: 'agent',
    instanceId: 'instance',
    matrixUserId: '@agent:test',
    provenance: 'autonomous_agent',
  },
  relation: { kind: 'reply', targetMessageId: request.messageId },
};
function receiver(): ReceiverView {
  return {
    running: true,
    failure: null,
    progress: { type: 'ready', agentId: 'agent', hostTaskId: 'task', roomId: request.roomId },
    state: {
      agentId: 'agent',
      roomCatalogId: 'catalog',
      instanceId: 'instance',
      bridgeService: 'test',
      enabled: true,
      checkpoint: { state: 'pending', afterEventId: null, eventId: request.matrixEventId },
      lastDelivery: {
        eventId: request.matrixEventId,
        messageId: request.messageId,
        submissionId: 'submission',
        stage: 'running',
        replyEventId: null,
        failure: null,
      },
      binding: {
        session: { sessionKey: 'session', displayName: 'Agent' },
        policy: { roomId: request.roomId, allowedPrincipalId: 'owner' },
        automationGrantId: 'grant',
        host: { taskId: 'task', workspace: 'workspace', executable: 'codex', mcpExecutable: 'mcp' },
        start: { mode: 'now' },
      },
    },
  };
}

describe('message evidence', () => {
  it('retains the configured task identity when starting the worker fails before registration', () => {
    const view = receiver();
    view.state.agentId = null;
    expect(receiverAgentId(view, [], [{ grantId: 'grant', agentId: 'agent' }])).toBe('agent');
    expect(receiverAgentId(view, [], [{ grantId: 'other', agentId: 'wrong-agent' }])).toBeNull();
  });
  it('never treats an unrelated message or human reply as agent receipt', () => {
    expect(
      messageDelivery(
        request,
        [
          { ...reply, relation: { kind: 'reply', targetMessageId: 'other' } },
          { ...reply, actor: request.actor },
        ],
        [],
      ),
    ).toEqual([]);
  });
  it('correlates agent, room, message and event, and handles multiple agents independently', () => {
    const view = receiver();
    expect(messageDelivery(request, [], [view])[0]?.stage).toBe('running');
    expect(messageDelivery({ ...request, roomId: '!other:test' }, [], [view])).toEqual([]);
    expect(messageDelivery({ ...request, matrixEventId: '$other' }, [], [view])).toEqual([]);
    expect(messageDelivery(request, [reply], [view])[0]?.stage).toBe('replied');
    expect(conversationDeliveries([request, reply], [view]).get(request.messageId)).toEqual(
      messageDelivery(request, [reply], [view]),
    );
  });
  it('does not leave a stopped worker claiming that a reply is in progress', () => {
    const view = receiver();
    view.running = false;
    expect(messageDelivery(request, [], [view])[0]?.stage).toBe('needs_review');
    expect(receiverStatus(view)).toBe('needs_review');
    view.running = true;
    view.progress = {
      type: 'reconnecting',
      error: { code: 'offline', category: 'transport', retryable: true, details: {} },
      retryInSeconds: 3,
    };
    expect(receiverStatus(view)).toBe('reconnecting');
  });
  it('requires a reply from this invited agent to this user after this invitation', () => {
    const target = {
      agentId: 'agent',
      principalId: 'owner',
      roomId: request.roomId,
      startedAt: 99,
    };
    expect(inviteReplyProgress([], target)).toBe('message');
    expect(inviteReplyProgress([request], target)).toBe('reply');
    expect(inviteReplyProgress([request, reply], target)).toBe('complete');
    expect(inviteReplyProgress([request, reply], { ...target, startedAt: 101 })).toBe('message');
    expect(
      inviteReplyProgress([request, reply], { ...target, agentId: 'same-name-different-agent' }),
    ).toBe('reply');
  });
});
