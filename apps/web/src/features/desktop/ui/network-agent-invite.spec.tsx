import { describe, expect, it } from 'vitest';

import type { PublicRoomSummary } from '@/features/room-directory/domain/public-room-directory';
import { networkInviteTarget } from './network-agent-invite';

const lobby: PublicRoomSummary = {
  catalogId: '0198b601-77a3-74f1-b4f4-940f291951b1',
  slug: 'agent-room-global',
  name: 'Agent Room Global',
  description: '',
  language: null,
  activeInstanceCount: 1,
  onlineAgentCount: 0,
};

describe('只凭网络接入能进哪里', () => {
  it('当前房间在公开大厅目录里就进这一间', () => {
    expect(networkInviteTarget(lobby.catalogId, [lobby])).toEqual({
      kind: 'lobby',
      name: 'Agent Room Global',
    });
  });

  it('目录里没有当前房间就是私人房间，网络 Agent 要凭口令进', () => {
    expect(networkInviteTarget('0198b601-77a3-74f1-b4f4-940f291951b2', [lobby])).toEqual({
      kind: 'private',
    });
  });

  it('没有房间上下文或还不知道目录时只说进公开大厅', () => {
    expect(networkInviteTarget(undefined, [lobby])).toEqual({ kind: 'lobby', name: null });
    expect(networkInviteTarget(lobby.catalogId, null)).toEqual({ kind: 'lobby', name: null });
  });
});
