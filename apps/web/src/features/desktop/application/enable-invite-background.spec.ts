import { describe, expect, it, vi } from 'vitest';

import type {
  AutomationGrant,
  AutomationGrantGateway,
} from '@/features/automation/domain/automation-grant';
import { err, ok } from '@/shared/result';
import type { HostSessionDiagnostics } from '../domain/desktop-runtime';
import type { ReceiverView } from '../domain/reception';
import { enableInviteBackground } from './enable-invite-background';

const agentId = '0198b601-77a1-7bb8-83eb-a8fe68c97e44';
const taskId = '0198b601-77a1-7bb8-83eb-a8fe68c97e45';
const roomCatalogId = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const grantId = '0198b601-77a1-7bb8-83eb-a8fe68c97e47';
const instanceId = '0198b601-77a1-7bb8-83eb-a8fe68c97e48';
const principalId = '0198b601-77a1-7bb8-83eb-a8fe68c97e49';
const session: HostSessionDiagnostics = {
  displayName: 'Scout',
  sessionKey: taskId,
  session: { sessionId: instanceId, state: 'ready', agentId, errorCode: null },
  receptionOffer: {
    task: { taskId, hostType: 'codex', workspace: 'C:\\project' },
    roomId: '!room:test',
    roomCatalogId,
    instanceId,
  },
  lastInboxReadAgoMs: 0,
  lastMessageReceivedAgoMs: null,
  lastMessageSentAgoMs: null,
};
const existing: AutomationGrant = {
  agentId,
  agentInstanceId: null,
  roomCatalogId,
  grantId: '0198b601-77a1-7bb8-83eb-a8fe68c97e50',
  audience: 'any_room_member',
  messageKinds: ['reply'],
  startsAtUnixMs: Date.now() - 1_000,
  expiresAtUnixMs: Date.now() + 86_400_000,
  maxMessagesPerMinute: 10,
  maxTotalMessages: 1000,
  requiresRiskScan: true,
  revokedAtUnixMs: null,
  status: 'active',
  messagesInCurrentMinute: 0,
  totalMessages: 0,
};

/** 绑定里只看得到任务的 sessionKey，别的字段这里用不到。 */
function bound(sessionKey: string): ReceiverView {
  return { state: { binding: { session: { sessionKey } } } } as unknown as ReceiverView;
}

function ports(
  options: { receivers?: readonly ReceiverView[]; grants?: readonly AutomationGrant[] } = {},
) {
  return {
    automation: {
      list: vi.fn<AutomationGrantGateway['list']>(() =>
        Promise.resolve(ok(options.grants ?? [existing])),
      ),
      create: vi.fn<AutomationGrantGateway['create']>(() => Promise.resolve(ok(existing))),
    },
    runtime: {
      listReceivers: vi.fn(() => Promise.resolve(ok(options.receivers ?? []))),
      configureReceiver: vi.fn(() => Promise.resolve(ok(undefined))),
      receiverAction: vi.fn(() => Promise.resolve(ok(undefined))),
    },
  };
}

const input = { session, principalId, grantId };

describe('接入对话框替人打开后台回复', () => {
  it('有能用的授权就接着用，绑定这个连接再启动', async () => {
    const dependencies = ports();
    expect(await enableInviteBackground(input, dependencies)).toEqual(ok(undefined));
    expect(dependencies.automation.create).not.toHaveBeenCalled();
    expect(dependencies.runtime.configureReceiver).toHaveBeenCalledWith({
      sessionId: instanceId,
      principalId,
      automationGrantId: existing.grantId,
      executable: null,
    });
    expect(dependencies.runtime.receiverAction).toHaveBeenCalledWith(taskId, { action: 'start' });
  });

  it('没有能用的授权：建一份 30 天、只许回复的', async () => {
    const dependencies = ports({ grants: [] });
    dependencies.automation.create.mockImplementation(() =>
      Promise.resolve(ok({ ...existing, grantId, agentInstanceId: instanceId })),
    );
    expect(await enableInviteBackground(input, dependencies)).toEqual(ok(undefined));
    expect(dependencies.automation.create).toHaveBeenCalledWith(
      grantId,
      expect.objectContaining({
        agentInstanceId: instanceId,
        messageKinds: ['reply'],
        lifetimeSeconds: 30 * 24 * 60 * 60,
      }),
    );
  });

  it('这个任务已经接上了后台回复：不再动', async () => {
    const dependencies = ports({ receivers: [bound(taskId)] });
    expect(await enableInviteBackground(input, dependencies)).toEqual(ok(undefined));
    expect(dependencies.automation.list).not.toHaveBeenCalled();
    expect(dependencies.runtime.configureReceiver).not.toHaveBeenCalled();
  });

  it('读不到授权：说没开成，不去绑定', async () => {
    const dependencies = ports();
    dependencies.automation.list.mockImplementation(() =>
      Promise.resolve(err({ code: 'automation.unavailable', retryable: true })),
    );
    expect(await enableInviteBackground(input, dependencies)).toEqual(
      err({ code: 'automation.unavailable', retryable: true }),
    );
    expect(dependencies.runtime.configureReceiver).not.toHaveBeenCalled();
  });

  it('这台电脑没有接收器（网页端）：开不了', async () => {
    const { automation, runtime } = ports();
    const result = await enableInviteBackground(input, {
      automation,
      runtime: {
        configureReceiver: runtime.configureReceiver,
        receiverAction: runtime.receiverAction,
      },
    });
    expect(result).toEqual(err({ code: 'receiver.unavailable' }));
    expect(automation.list).not.toHaveBeenCalled();
  });
});
