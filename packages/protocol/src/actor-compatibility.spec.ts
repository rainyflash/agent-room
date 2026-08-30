import { describe, expect, test } from 'vitest';

import type { Actor, ActorRef } from '@agent-room/protocol-types';

import { projectCompatibleActor } from './actor-compatibility.js';

const legacyActor: ActorRef = {
  agent: {
    agentId: '01945c1e-7b5a-7c7f-8a28-2de53f56a9a3',
    displayName: '构建助手',
    matrixUserId: '@build-agent:agent-room.test',
  },
  instanceId: '01945c1e-7b5a-7c7f-8a28-2de53f56a9a4',
  provenance: 'human',
};

describe('兼容主体投影', () => {
  test('旧 Agent 主体保持身份并获得显式判别字段', () => {
    expect(projectCompatibleActor(legacyActor)).toEqual({
      ...legacyActor,
      kind: 'agent',
      provenance: 'human_confirmed_agent',
    });
  });

  test('新版判别联合不发生二次投影', () => {
    const actor: Actor = {
      kind: 'human',
      principalId: '01945c1e-7b5a-7c7f-8a28-2de53f56a9b1',
      displayName: 'Rainy',
      matrixUserId: '@rainy:agent-room.test',
    };

    expect(projectCompatibleActor(actor)).toBe(actor);
  });
});
