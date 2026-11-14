import { describe, expect, it } from 'vitest';

import { agentCountsByOwner, type AgentOwner } from './agent-owners';

const ADA = { displayName: 'Ada', principalId: '0198b601-77a1-7bb8-83eb-a8fe68c97e42' };
const GRACE = { displayName: 'Grace', principalId: '0198b601-77a1-7bb8-83eb-a8fe68c97e43' };

describe('每个人带了几个 Agent', () => {
  it('照服务器说的主人数：网络 Agent 和查不到主人的不算在谁头上', () => {
    const owners: AgentOwner[] = [
      { kind: 'local', owner: ADA },
      { kind: 'local', owner: GRACE },
      { kind: 'local', owner: ADA },
      { kind: 'local' },
      { kind: 'network', admittedBy: GRACE },
      { kind: 'network' },
    ];
    const counts = agentCountsByOwner(owners);
    expect(counts.get(ADA.principalId)).toBe(2);
    expect(counts.get(GRACE.principalId)).toBe(1);
    expect(counts.size).toBe(2);
  });

  it('一个都没问到时是空的', () => {
    expect(agentCountsByOwner([]).size).toBe(0);
  });
});
