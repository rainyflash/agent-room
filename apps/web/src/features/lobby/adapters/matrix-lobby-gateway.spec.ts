import { describe, expect, it, vi } from 'vitest';

import { MatrixLobbyGateway } from './matrix-lobby-gateway';
import type {
  MatrixLobbyRoomSnapshot,
  MatrixLobbySource,
  MatrixLobbySourceRead,
  MatrixLobbyStateEvent,
} from './matrix-lobby-source';

const NOW = Date.parse('2026-08-24T16:00:00.000Z');
const AGENT_ID = '01990d9e-8400-7000-8000-000000000001';
const MATRIX_USER_ID = '@build-agent:agent-room.test';

describe('MatrixLobbyGateway', () => {
  it('把 Matrix 生命周期故障映射为闭合的大厅失败', () => {
    const unavailable = new MatrixLobbyGateway(source({ kind: 'matrix-unavailable' }), () => NOW);
    const missing = new MatrixLobbyGateway(source({ kind: 'room-not-joined' }), () => NOW);

    expect(unavailable.read('!room:agent-room.test')).toEqual({
      error: { code: 'lobby.matrix_unavailable', retryable: true },
      ok: false,
    });
    expect(missing.read('!room:agent-room.test')).toEqual({
      error: { code: 'lobby.room_not_joined', retryable: true },
      ok: false,
    });
  });

  it('按 Agent 聚合多实例，并让需要关注的有效状态取得代表权', () => {
    const room = snapshot([
      statusState({ instanceSuffix: '1', status: 'working', summary: '正在构建索引' }),
      statusState({ instanceSuffix: '2', status: 'blocked', summary: '等待仓库权限' }),
    ]);
    const gateway = new MatrixLobbyGateway(source({ kind: 'ready', room }), () => NOW);

    const result = gateway.read(room.roomId);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value).toMatchObject({
      name: '公开大厅',
      observedAtUnixMs: NOW,
      roomId: room.roomId,
      topic: '协作工作区',
    });
    expect(result.value.agents).toEqual([
      {
        agentId: AGENT_ID,
        displayName: '构建助手',
        instanceIds: [
          '01990d9e-8400-7000-8000-000000000011',
          '01990d9e-8400-7000-8000-000000000012',
        ],
        matrixUserId: MATRIX_USER_ID,
        status: 'blocked',
        statusExpiresAtUnixMs: Date.parse('2026-08-24T16:00:30.000Z') + 15_000,
        summary: '等待仓库权限',
        trust: 'unknown',
        visibility: 'detailed',
      },
    ]);
    expect(Object.isFrozen(result.value)).toBe(true);
    expect(Object.isFrozen(result.value.agents)).toBe(true);
  });

  it('把过期租约投影为离线，并隔离伪造、冲突和畸形状态', () => {
    const validExpired = statusState({
      createdAt: '2026-08-24T15:54:00.000Z',
      instanceSuffix: '1',
      leaseExpiresAt: '2026-08-24T15:58:00.000Z',
      status: 'working',
      summary: '已失联',
    });
    const wrongSender = {
      ...statusState({ instanceSuffix: '2', status: 'blocked' }),
      sender: '@attacker:agent-room.test',
    };
    const wrongStateKey = {
      ...statusState({ instanceSuffix: '3', status: 'working' }),
      stateKey: 'not-the-instance-id',
    };
    const coarseLeak = statusState({ instanceSuffix: '4', status: 'working' });
    coarseLeak.content = {
      ...statusContent({ instanceSuffix: '4', status: 'working' }),
      visibility: 'coarse',
    };
    const gateway = new MatrixLobbyGateway(
      source({
        kind: 'ready',
        room: snapshot([validExpired, wrongSender, wrongStateKey, coarseLeak]),
      }),
      () => NOW,
    );

    const result = gateway.read('!public:agent-room.test');

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.agents).toHaveLength(1);
    expect(result.value.agents[0]).toMatchObject({
      agentId: AGENT_ID,
      instanceIds: ['01990d9e-8400-7000-8000-000000000011'],
      status: 'offline',
    });
  });

  it('把订阅生命周期完整委托给 Matrix Source', () => {
    const unsubscribe = vi.fn();
    const subscribe = vi.fn(() => unsubscribe);
    const lobbySource: MatrixLobbySource = {
      read: () => ({ kind: 'matrix-unavailable' }),
      subscribe,
    };
    const gateway = new MatrixLobbyGateway(lobbySource, () => NOW);
    const listener = vi.fn();

    const detach = gateway.subscribe('!public:agent-room.test', listener);
    detach();

    expect(subscribe).toHaveBeenCalledWith('!public:agent-room.test', listener);
    expect(unsubscribe).toHaveBeenCalledOnce();
  });
});

function source(read: MatrixLobbySourceRead): MatrixLobbySource {
  return {
    read: () => read,
    subscribe: () => noop,
  };
}

function noop(): void {
  return undefined;
}

function snapshot(statusEvents: readonly MatrixLobbyStateEvent[]): MatrixLobbyRoomSnapshot {
  return {
    joinedMemberIds: [MATRIX_USER_ID],
    name: '公开大厅',
    roomId: '!public:agent-room.test',
    statusEvents,
    topic: '协作工作区',
  };
}

function statusState(options: StatusOptions): MatrixLobbyStateEvent & { content: unknown } {
  const content = statusContent(options);
  return {
    content,
    sender: MATRIX_USER_ID,
    stateKey: content.actor.instanceId,
  };
}

type StatusOptions = {
  readonly createdAt?: string;
  readonly instanceSuffix: string;
  readonly leaseExpiresAt?: string;
  readonly status: 'blocked' | 'working';
  readonly summary?: string;
};

function statusContent(options: StatusOptions) {
  const instanceId = `01990d9e-8400-7000-8000-00000000001${options.instanceSuffix}`;
  return {
    actor: {
      agent: {
        agentId: AGENT_ID,
        displayName: '构建助手',
        matrixUserId: MATRIX_USER_ID,
      },
      instanceId,
      provenance: 'autonomous_agent',
    },
    correlationId: '01990d9e-8400-7000-8000-000000000090',
    createdAt: options.createdAt ?? '2026-08-24T16:00:00.000Z',
    eventType: 'org.agentroom.agent.status.v1',
    id: `01990d9e-8400-7000-8000-00000000002${options.instanceSuffix}`,
    leaseExpiresAt: options.leaseExpiresAt ?? '2026-08-24T16:00:30.000Z',
    progress: 0.5,
    schemaVersion: '1.0',
    signature: 'BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB',
    startedAt: '2026-08-24T15:59:00.000Z',
    status: options.status,
    taskSummary: options.summary ?? '处理中',
    visibility: 'detailed',
  };
}
