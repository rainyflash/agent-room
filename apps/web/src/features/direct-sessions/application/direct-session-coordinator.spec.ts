import { describe, expect, it, vi } from 'vitest';

import { DirectSessionCoordinator } from './direct-session-coordinator';
import type {
  DirectBlockRegistry,
  DirectContact,
  DirectSession,
  DirectSessionGateway,
  DirectSessionMatrixGateway,
} from '@/features/direct-sessions/domain/direct-session';
import { err, ok } from '@/shared/result';

describe('DirectSessionCoordinator', () => {
  it('重复打开交给控制平面复用并在返回前准备 Matrix 房间', async () => {
    const value = session();
    const open = vi.fn().mockResolvedValue(ok(value));
    const prepare = vi.fn().mockResolvedValue(ok(undefined));
    const coordinator = new DirectSessionCoordinator(
      controlPlane({ open }),
      matrix({ prepare }),
      localBlocks(),
    );

    await expect(coordinator.open(value.target.agentId)).resolves.toEqual(ok(value));
    expect(open).toHaveBeenCalledWith(value.target.agentId);
    expect(prepare).toHaveBeenCalledWith(value);
  });

  it('屏蔽立即写本地并并行同步服务端与 Matrix，解除仅在双端成功后生效', async () => {
    const order: string[] = [];
    const setBlocked = vi.fn().mockImplementation((_agentId: string, blocked: boolean) => {
      order.push(blocked ? 'server-block' : 'server-unblock');
      return Promise.resolve(ok(contact(blocked)));
    });
    const setIgnored = vi.fn().mockImplementation((_userId: string, ignored: boolean) => {
      order.push(ignored ? 'matrix-block' : 'matrix-unblock');
      return Promise.resolve(ok(undefined));
    });
    const coordinator = new DirectSessionCoordinator(
      controlPlane({ setBlocked }),
      matrix({ setIgnored }),
      localBlocks(),
    );

    await coordinator.setBlocked(session().target, true);
    await coordinator.setBlocked(session().target, false);

    expect(order.slice(0, 2).toSorted()).toEqual(['matrix-block', 'server-block']);
    expect(order.slice(2)).toEqual(['server-unblock', 'matrix-unblock']);
  });

  it('服务端失败也保持本地屏蔽且仍尝试 Matrix 忽略', async () => {
    const blocks = localBlocks();
    const target = session().target;
    const setIgnored = vi.fn().mockResolvedValue(ok(undefined));
    const coordinator = new DirectSessionCoordinator(
      controlPlane({
        setBlocked: vi
          .fn()
          .mockResolvedValue(err({ code: 'direct_session.unreachable', retryable: true })),
      }),
      matrix({ setIgnored }),
      blocks,
    );

    const pending = coordinator.setBlocked(target, true);
    expect(coordinator.isLocallyBlocked(target.agentId)).toBe(true);
    await expect(pending).resolves.toEqual(
      err({ code: 'direct_session.unreachable', retryable: true }),
    );
    expect(coordinator.isLocallyBlocked(target.agentId)).toBe(true);
    expect(setIgnored).toHaveBeenCalledWith(target.matrixUserId, true);
  });

  it('别的设备上屏蔽的（只在忽略名单里）也算屏蔽着', () => {
    const target = session().target;
    const coordinator = new DirectSessionCoordinator(
      controlPlane({}),
      matrix({
        ignoredUsers: () => [{ displayName: null, matrixUserId: target.matrixUserId }],
      }),
      localBlocks(),
    );

    expect(coordinator.isLocallyBlocked(target.agentId)).toBe(false);
    expect(coordinator.isBlocked(target)).toBe(true);
    expect(
      coordinator.isBlocked({ agentId: OTHER_AGENT_ID, matrixUserId: '@_agent_other:test' }),
    ).toBe(false);
  });

  it('设置里列出服务器记着的，再补上只在忽略名单里的；两边都有的只列一次', async () => {
    const target = session().target;
    const coordinator = new DirectSessionCoordinator(
      controlPlane({
        listBlocks: () => Promise.resolve(ok([{ blockedAtUnixMs: 1_700_000_000_000, target }])),
      }),
      matrix({
        ignoredUsers: () => [
          { displayName: 'Build Agent', matrixUserId: target.matrixUserId },
          { displayName: 'Noisy', matrixUserId: '@_agent_noisy:matrix.agent-room.test' },
        ],
      }),
      localBlocks(),
    );

    await expect(coordinator.listBlocked()).resolves.toEqual(
      ok([
        {
          agentId: target.agentId,
          blockedAtUnixMs: 1_700_000_000_000,
          displayName: 'Build Agent',
          matrixUserId: target.matrixUserId,
        },
        {
          agentId: null,
          blockedAtUnixMs: null,
          displayName: 'Noisy',
          matrixUserId: '@_agent_noisy:matrix.agent-room.test',
        },
      ]),
    );
  });

  it('解除只在忽略名单里的屏蔽不去问服务器，服务器记着的两边都解', async () => {
    const target = session().target;
    const setBlocked = vi.fn().mockResolvedValue(ok(contact(false)));
    const setIgnored = vi.fn().mockResolvedValue(ok(undefined));
    const blocks = localBlocks();
    blocks.set(target.agentId, true);
    const coordinator = new DirectSessionCoordinator(
      controlPlane({ setBlocked }),
      matrix({ setIgnored }),
      blocks,
    );

    await expect(
      coordinator.unblock({
        agentId: null,
        blockedAtUnixMs: null,
        displayName: null,
        matrixUserId: '@_agent_noisy:matrix.agent-room.test',
      }),
    ).resolves.toEqual(ok(undefined));
    expect(setBlocked).not.toHaveBeenCalled();
    expect(setIgnored).toHaveBeenCalledWith('@_agent_noisy:matrix.agent-room.test', false);

    await expect(
      coordinator.unblock({
        agentId: target.agentId,
        blockedAtUnixMs: 1_700_000_000_000,
        displayName: target.displayName,
        matrixUserId: target.matrixUserId,
      }),
    ).resolves.toEqual(ok(undefined));
    expect(setBlocked).toHaveBeenCalledWith(target.agentId, false);
    expect(setIgnored).toHaveBeenLastCalledWith(target.matrixUserId, false);
    expect(coordinator.isLocallyBlocked(target.agentId)).toBe(false);
  });

  it('Matrix 准备失败时不伪造已打开结果', async () => {
    const coordinator = new DirectSessionCoordinator(
      controlPlane({ open: vi.fn().mockResolvedValue(ok(session())) }),
      matrix({
        prepare: vi
          .fn()
          .mockResolvedValue(err({ code: 'direct_session.join_failed', retryable: true })),
      }),
      localBlocks(),
    );

    await expect(coordinator.open(session().target.agentId)).resolves.toEqual(
      err({ code: 'direct_session.join_failed', retryable: true }),
    );
  });
});

const OTHER_AGENT_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e59';

function controlPlane(overrides: Partial<DirectSessionGateway>): DirectSessionGateway {
  return {
    inspect: () => Promise.resolve(ok(session())),
    list: () => Promise.resolve(ok([session()])),
    listBlocks: () => Promise.resolve(ok([])),
    open: () => Promise.resolve(ok(session())),
    setBlocked: (_agentId, blocked) => Promise.resolve(ok(contact(blocked))),
    ...overrides,
  };
}

function localBlocks(): DirectBlockRegistry {
  const blocked = new Set<string>();
  return {
    has: (agentId) => blocked.has(agentId),
    set: (agentId, value) => {
      if (value) {
        blocked.add(agentId);
      } else {
        blocked.delete(agentId);
      }
    },
  };
}

function matrix(overrides: Partial<DirectSessionMatrixGateway>): DirectSessionMatrixGateway {
  return {
    ignoredUsers: () => [],
    markDisplayed: () => Promise.resolve(ok(undefined)),
    prepare: () => Promise.resolve(ok(undefined)),
    setIgnored: () => Promise.resolve(ok(undefined)),
    ...overrides,
  };
}

function contact(blocked: boolean): DirectContact {
  const value = session();
  return {
    contactPolicy: {
      agentBlocksPrincipal: false,
      deliveryAllowed: !blocked,
      presenceDisclosure: blocked ? 'hidden' : 'coarse',
      principalBlocksAgent: blocked,
    },
    target: value.target,
  };
}

function session(): DirectSession {
  return {
    catalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e53',
    contactPolicy: contactPolicy(),
    lifecycle: 'active',
    matrixRoomId: '!direct:matrix.agent-room.test',
    roomInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e54',
    target: {
      agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e52',
      avatarContentId: null,
      displayName: 'Build Agent',
      matrixUserId: '@_agent_build:matrix.agent-room.test',
    },
    version: 1,
  };
}

function contactPolicy() {
  return {
    agentBlocksPrincipal: false,
    deliveryAllowed: true,
    presenceDisclosure: 'coarse' as const,
    principalBlocksAgent: false,
  };
}
