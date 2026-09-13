import { describe, expect, it, vi } from 'vitest';
import { enableReception } from './enable-reception';
import type { AutomationGrant } from '@/features/automation/domain/automation-grant';
import type { HostSessionDiagnostics } from '../domain/desktop-runtime';
import { err, ok } from '@/shared/result';

const agentId = '0198b601-77a1-7bb8-83eb-a8fe68c97e44';
const taskId = '0198b601-77a1-7bb8-83eb-a8fe68c97e45';
const roomCatalogId = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const grantId = '0198b601-77a1-7bb8-83eb-a8fe68c97e47';
const instanceId = '0198b601-77a1-7bb8-83eb-a8fe68c97e48';
const principalId = '0198b601-77a1-7bb8-83eb-a8fe68c97e49';
const session: HostSessionDiagnostics = {
  displayName: 'Scout',
  sessionKey: taskId,
  session: { sessionId: taskId, state: 'ready', agentId, errorCode: null },
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
const grant: AutomationGrant = {
  agentId,
  agentInstanceId: instanceId,
  roomCatalogId,
  grantId,
  audience: 'known_room_members',
  messageKinds: ['reply'],
  startsAtUnixMs: 100,
  expiresAtUnixMs: 100000,
  maxMessagesPerMinute: 10,
  maxTotalMessages: 1000,
  requiresRiskScan: true,
  revokedAtUnixMs: null,
  status: 'active',
  messagesInCurrentMinute: 0,
  totalMessages: 0,
};
const grants: readonly AutomationGrant[] = [];
const input = { session, principalId, grantId, grants, now: 200 };
function dependencies() {
  const calls: string[] = [];
  return {
    calls,
    automation: {
      create: vi.fn(() => {
        calls.push('authorize');
        return Promise.resolve(ok(grant));
      }),
    },
    runtime: {
      configureReceiver: vi.fn(() => {
        calls.push('configure');
        return Promise.resolve(ok(undefined));
      }),
      receiverAction: vi.fn(() => {
        calls.push('start');
        return Promise.resolve(ok(undefined));
      }),
    },
  };
}
describe('enable background replies', () => {
  it('一次明确操作按顺序完成房间授权、绑定与启动', async () => {
    const ports = dependencies();
    expect(await enableReception(input, ports)).toEqual(ok(undefined));
    expect(ports.calls).toEqual(['authorize', 'configure', 'start']);
    expect(ports.automation.create).toHaveBeenCalledWith(
      grantId,
      expect.objectContaining({
        agentId,
        agentInstanceId: instanceId,
        roomCatalogId,
        messageKinds: ['reply'],
        impactAcknowledged: true,
      }),
    );
    expect(ports.runtime.receiverAction).toHaveBeenCalledWith(taskId, { action: 'start' });
  });
  it('复用已有有效授权，避免每次创建新的授权', async () => {
    const ports = dependencies();
    expect(await enableReception({ ...input, grants: [grant] }, ports)).toEqual(ok(undefined));
    expect(ports.calls).toEqual(['configure', 'start']);
  });
  it('创建授权使用服务器返回的起始时间，网络耗时不误判为未生效', async () => {
    const ports = dependencies();
    ports.automation.create.mockResolvedValueOnce(
      ok({ ...grant, startsAtUnixMs: input.now + 100 }),
    );
    expect(await enableReception(input, ports)).toEqual(ok(undefined));
  });
  it('授权失败和绑定失败都不能继续启动任务', async () => {
    const ports = dependencies();
    const denied = err({ code: 'test.denied', retryable: false });
    expect(
      await enableReception(input, {
        ...ports,
        automation: { create: () => Promise.resolve(denied) },
      }),
    ).toEqual(denied);
    expect(ports.runtime.configureReceiver).not.toHaveBeenCalled();
    expect(
      await enableReception(input, {
        ...ports,
        runtime: { ...ports.runtime, configureReceiver: () => Promise.resolve(denied) },
      }),
    ).toEqual(denied);
    expect(ports.runtime.receiverAction).not.toHaveBeenCalled();
  });
  it('不接受其他房间的授权，也不启动尚未就绪的任务', async () => {
    const ports = dependencies();
    ports.automation.create.mockResolvedValueOnce(ok({ ...grant, roomCatalogId: taskId }));
    expect(await enableReception(input, ports)).toEqual(
      err({ code: 'receiver.authorization_mismatch', retryable: false }),
    );
    expect(ports.runtime.configureReceiver).not.toHaveBeenCalled();
    const stopped = { ...session, session: { ...session.session, state: 'closed' as const } };
    expect(await enableReception({ ...input, session: stopped }, ports)).toEqual(
      err({ code: 'receiver.session_not_ready', retryable: true }),
    );
  });
});
