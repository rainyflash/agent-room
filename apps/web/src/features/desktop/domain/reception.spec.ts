import { describe, expect, it } from 'vitest';
import { automationGrantSchema } from '@/features/automation/domain/automation-grant';
import { receptionFailureAdvice, receptionGrants } from './reception';

const identity = '0198b601-77a1-7bb8-83eb-a8fe68c97e44';
const other = '0198b601-77a1-7bb8-83eb-a8fe68c97e45';
const grant = automationGrantSchema.parse({
  agentId: identity,
  agentInstanceId: null,
  roomCatalogId: identity,
  grantId: identity,
  audience: 'known_room_members',
  expiresAtUnixMs: 3000,
  startsAtUnixMs: 1000,
  maxMessagesPerMinute: 5,
  maxTotalMessages: 100,
  messageKinds: ['reply'],
  messagesInCurrentMinute: 0,
  requiresRiskScan: false,
  revokedAtUnixMs: null,
  status: 'active',
  totalMessages: 0,
});

describe('receptionGrants', () => {
  it('only offers current reply grants for the exact agent, room and instance', () => {
    const rejected = [
      { ...grant, agentId: other },
      { ...grant, roomCatalogId: other },
      { ...grant, agentInstanceId: other },
      { ...grant, startsAtUnixMs: 2500 },
      { ...grant, expiresAtUnixMs: 1500 },
      { ...grant, totalMessages: 100 },
      { ...grant, status: 'revoked' as const, revokedAtUnixMs: 1500 },
      { ...grant, messageKinds: ['room_message'] as const },
    ];
    expect(receptionGrants([grant, ...rejected], identity, identity, identity, 2000)).toEqual([
      grant,
    ]);
    expect(receptionGrants([grant], null, identity, identity, 2000)).toEqual([]);
    expect(receptionGrants([grant], identity, null, identity, 2000)).toEqual([]);
  });
});

describe('reception failure advice', () => {
  it('把 receiver.* 错误码归到人能处理的几类，未知代码给通用提示', () => {
    expect(receptionFailureAdvice('receiver.host_turn_failed')).toBe('host');
    expect(receptionFailureAdvice('receiver.host_output_invalid')).toBe('host');
    expect(receptionFailureAdvice('receiver.executable_invalid')).toBe('host');
    expect(receptionFailureAdvice('receiver.storage_unavailable')).toBe('files');
    expect(receptionFailureAdvice('receiver.checkpoint_write_failed')).toBe('files');
    expect(receptionFailureAdvice('receiver.automation_grant_invalid')).toBe('authorization');
    expect(receptionFailureAdvice('receiver.task_id_invalid')).toBe('session');
    expect(receptionFailureAdvice('receiver.session_not_ready')).toBe('session');
    expect(receptionFailureAdvice('receiver.pending_review_required')).toBe('review');
    expect(receptionFailureAdvice('receiver.stop_timeout')).toBe('unresponsive');
    expect(receptionFailureAdvice('receiver.worker_failed')).toBe('unresponsive');
    expect(receptionFailureAdvice('receiver.something_new')).toBe('unknown');
    expect(receptionFailureAdvice('bridge.ipc.bridge_unavailable')).toBe('unknown');
  });
});
