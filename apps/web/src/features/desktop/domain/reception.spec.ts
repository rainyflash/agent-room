import { describe, expect, it } from 'vitest';
import { automationGrantSchema } from '@/features/automation/domain/automation-grant';
import { receptionGrants } from './reception';

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
