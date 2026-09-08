import type { DesktopRuntimeGateway } from '@/features/desktop/domain/desktop-runtime';
import { receiverViewSchema, type ReceiverView } from '@/features/desktop/domain/reception';
import { automationGrantSchema } from '@/features/automation/domain/automation-grant';
import { err, ok } from '@/shared/result';

export function receptionFixture(
  agentId: string,
  catalogId: string,
  principalId: string,
  instanceId: string,
) {
  const grant = automationGrantSchema.parse({
    agentId,
    agentInstanceId: null,
    roomCatalogId: catalogId,
    grantId: '0198b601-77a4-74f1-b4f4-940f291951b9',
    audience: 'known_room_members',
    expiresAtUnixMs: Date.now() + 86400000,
    startsAtUnixMs: Date.now() - 1000,
    maxMessagesPerMinute: 5,
    maxTotalMessages: 100,
    messageKinds: ['reply'],
    messagesInCurrentMinute: 0,
    requiresRiskScan: false,
    revokedAtUnixMs: null,
    status: 'active',
    totalMessages: 0,
  });
  const taskId = '0198b601-77a5-74f1-b4f4-940f291951b9';
  const initial = receiverViewSchema.parse({
    running: false,
    progress: null,
    failure: null,
    state: {
      binding: {
        session: { sessionKey: taskId, displayName: 'Reception Scout' },
        policy: { roomId: '!fixture:matrix.test', allowedPrincipalId: principalId },
        automationGrantId: grant.grantId,
        host: {
          taskId,
          executable: 'C:/Agent Tools/codex.exe',
          mcpExecutable: 'C:/Agent Room/agent-room-mcp.exe',
          workspace: 'C:/Projects/Studio',
        },
        start: { mode: 'now' },
      },
      bridgeService: 'fixture.reception',
      agentId,
      roomCatalogId: catalogId,
      instanceId,
      checkpoint: { state: 'ready', afterEventId: null },
      enabled: false,
      lastDelivery: null,
    },
  });
  let views: ReceiverView[] = [];
  const fail = () => err({ code: 'fixture.receiver_invalid', retryable: false });
  const gateway: Pick<
    DesktopRuntimeGateway,
    'listReceivers' | 'configureReceiver' | 'receiverAction'
  > = {
    listReceivers: () => Promise.resolve(ok(structuredClone(views))),
    configureReceiver: (request) => {
      if (
        request.principalId !== principalId ||
        request.automationGrantId !== grant.grantId ||
        request.sessionId !== instanceId
      )
        return Promise.resolve(fail());
      views = [structuredClone(initial)];
      return Promise.resolve(ok(undefined));
    },
    receiverAction: (id, request) => {
      const view = views.find((entry) => entry.state.binding.host.taskId === id);
      if (!view) return Promise.resolve(fail());
      switch (request.action) {
        case 'start':
          view.running = true;
          view.state.enabled = true;
          break;
        case 'pause':
          view.running = false;
          view.state.enabled = false;
          break;
        case 'remove':
          views = [];
          break;
        case 'update':
          view.state.binding.automationGrantId = request.automationGrantId;
          if (request.workspace) view.state.binding.host.workspace = request.workspace;
          if (request.executable) view.state.binding.host.executable = request.executable;
          break;
        case 'verify':
          view.state.checkpoint = { state: 'ready', afterEventId: '$input' };
          if (view.state.lastDelivery) {
            view.state.lastDelivery.stage = 'replied';
            view.state.lastDelivery.failure = null;
            view.state.lastDelivery.replyEventId = '$reply';
          }
          break;
        case 'resolve':
          view.state.checkpoint = {
            state: 'ready',
            afterEventId: request.resolution === 'skip' ? '$input' : null,
          };
          view.running = request.resolution === 'retry';
          break;
      }
      return Promise.resolve(ok(undefined));
    },
  };
  if (new URLSearchParams(location.search).get('reception') === 'pending') {
    const view = structuredClone(initial);
    view.state.checkpoint = { state: 'pending', afterEventId: null, eventId: '$input' };
    view.state.lastDelivery = {
      eventId: '$input',
      messageId: taskId,
      submissionId: grant.grantId,
      stage: 'needs_review',
      replyEventId: null,
      failure: {
        code: 'receiver.reply_unconfirmed',
        category: 'internal',
        retryable: false,
        details: {},
      },
    };
    views = [view];
  }
  return {
    gateway,
    grant,
    sessionKey: taskId,
    offer: {
      task: { taskId, workspace: 'C:/Projects/Studio' },
      roomId: '!fixture:matrix.test',
      roomCatalogId: catalogId,
      instanceId,
    },
  };
}
