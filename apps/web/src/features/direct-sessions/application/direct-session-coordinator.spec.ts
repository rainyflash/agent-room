import { describe, expect, it, vi } from 'vitest';

import { DirectSessionCoordinator } from './direct-session-coordinator';
import type {
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
    const coordinator = new DirectSessionCoordinator(controlPlane({ open }), matrix({ prepare }));

    await expect(coordinator.open(value.target.agentId)).resolves.toEqual(ok(value));
    expect(open).toHaveBeenCalledWith(value.target.agentId);
    expect(prepare).toHaveBeenCalledWith(value);
  });

  it('屏蔽先写服务端再写 Matrix，解除时采用相反顺序', async () => {
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
    );

    await coordinator.setBlocked(session().target, true);
    await coordinator.setBlocked(session().target, false);

    expect(order).toEqual(['server-block', 'matrix-block', 'matrix-unblock', 'server-unblock']);
  });

  it('Matrix 准备失败时不伪造已打开结果', async () => {
    const coordinator = new DirectSessionCoordinator(
      controlPlane({ open: vi.fn().mockResolvedValue(ok(session())) }),
      matrix({
        prepare: vi
          .fn()
          .mockResolvedValue(err({ code: 'direct_session.join_failed', retryable: true })),
      }),
    );

    await expect(coordinator.open(session().target.agentId)).resolves.toEqual(
      err({ code: 'direct_session.join_failed', retryable: true }),
    );
  });
});

function controlPlane(overrides: Partial<DirectSessionGateway>): DirectSessionGateway {
  return {
    inspect: () => Promise.resolve(ok(session())),
    list: () => Promise.resolve(ok([session()])),
    open: () => Promise.resolve(ok(session())),
    setBlocked: (_agentId, blocked) => Promise.resolve(ok(contact(blocked))),
    ...overrides,
  };
}

function matrix(overrides: Partial<DirectSessionMatrixGateway>): DirectSessionMatrixGateway {
  return {
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
