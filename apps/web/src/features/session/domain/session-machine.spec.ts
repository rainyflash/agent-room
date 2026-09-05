import { createActor, waitFor } from 'xstate';
import { describe, expect, it, vi } from 'vitest';

import type {
  BrowserGateway,
  ControlPlaneGateway,
  MatrixConnection,
  MatrixGateway,
  SessionDependencies,
  SessionFailure,
  WebSession,
} from './session';
import { createSessionMachine } from './session-machine';
import { err, ok } from '@/shared/result';

const session: WebSession = {
  authenticatedAtUnixMs: 1_700_000_000_000,
  displayName: 'Local Developer',
  expiresAtUnixMs: 1_700_028_800_000,
  locale: 'en',
  matrixUserId: '@user-0123456789abcdef:matrix.agent-room.test',
  principalId: '018c251e-7b5a-7c7f-8a28-2de53f56a9a3',
  recentlyAuthenticated: true,
};

function connection(): MatrixConnection {
  return {
    deviceId: 'WEBDEVICE',
    userId: session.matrixUserId,
    disconnect: () => undefined,
    observe: () => () => undefined,
    waitUntilPrepared: () => Promise.resolve(ok(undefined)),
  };
}

function dependencies(
  overrides: {
    readonly controlPlane?: Partial<ControlPlaneGateway>;
    readonly matrix?: Partial<MatrixGateway>;
  } = {},
) {
  const navigations: string[] = [];
  const browser: BrowserGateway = {
    currentPath: () => '/lobby/public?directory=open',
    isOnline: () => true,
    replacePath: (path) => {
      navigations.push(path);
    },
  };
  const controlPlane: ControlPlaneGateway = {
    beginAuthentication: () => Promise.resolve(ok({ kind: 'browser-navigation' })),
    logout: () => Promise.resolve(ok(undefined)),
    readSession: () => Promise.resolve(ok(session)),
    ...overrides.controlPlane,
  };
  const matrix: MatrixGateway = {
    disconnect: vi.fn(),
    beginAuthentication: () => Promise.resolve(ok({ kind: 'browser-navigation' })),
    logout: () => Promise.resolve(ok(undefined)),
    restore: () =>
      Promise.resolve(
        ok({
          connection: connection(),
          kind: 'connected',
          returnPath: '/lobby/public?directory=open',
        }),
      ),
    ...overrides.matrix,
  };
  const value: SessionDependencies = {
    browser,
    controlPlane,
    matrix,
    privateState: { clear: vi.fn() },
  };
  return { navigations, value };
}

describe('Web 会话状态机', () => {
  it('依赖健康报告降级不会撤销已经验证的云端账户能力', async () => {
    const runtime = dependencies();
    const actor = createActor(createSessionMachine(runtime.value)).start();
    await waitFor(actor, (snapshot) => snapshot.matches('ready'));
    actor.send({
      type: 'CONTROL_DEGRADED',
      reachable: true,
      failure: {
        boundary: 'control-plane',
        code: 'control_plane.readiness_degraded',
        offline: false,
        retryable: true,
      },
    });
    expect(actor.getSnapshot().matches('degraded')).toBe(true);
    expect(actor.getSnapshot().context.controlStatus).toBe('ready');
    expect(actor.getSnapshot().context.principal).toEqual(session);
    actor.stop();
  });

  it('只有云端账户可用而 Matrix 尚未登录时也能退出', async () => {
    const logout = vi.fn<ControlPlaneGateway['logout']>().mockResolvedValue(ok(undefined));
    const runtime = dependencies({
      controlPlane: { logout },
      matrix: { restore: () => Promise.resolve(ok({ kind: 'authentication-required' })) },
    });
    const actor = createActor(createSessionMachine(runtime.value)).start();
    await waitFor(actor, (snapshot) => snapshot.matches('unauthenticated'));
    expect(actor.getSnapshot().context.controlStatus).toBe('ready');

    actor.send({ type: 'LOGOUT' });
    await waitFor(actor, (snapshot) => snapshot.matches('unauthenticated'));
    expect(logout).toHaveBeenCalledOnce();
    expect(actor.getSnapshot().context.principal).toBeNull();
    expect(actor.getSnapshot().context.controlStatus).toBe('unauthenticated');
    actor.stop();
  });

  it('退出失败会锁定本地工作区并保留重试，不能重新联网后静默恢复旧会话', async () => {
    const logout = vi
      .fn<ControlPlaneGateway['logout']>()
      .mockResolvedValueOnce(
        err({
          boundary: 'control-plane',
          code: 'control_plane.unreachable',
          offline: true,
          retryable: true,
        }),
      )
      .mockResolvedValueOnce(ok(undefined));
    const runtime = dependencies({ controlPlane: { logout } });
    const actor = createActor(createSessionMachine(runtime.value)).start();
    await waitFor(actor, (snapshot) => snapshot.matches('ready'));

    actor.send({ type: 'LOGOUT' });
    const failed = await waitFor(actor, (snapshot) => snapshot.matches('signOutFailed'));
    expect(failed.context.principal).toBeNull();
    expect(failed.context.connection).toBeNull();
    expect(failed.context.controlStatus).toBe('unauthenticated');
    expect(failed.context.failure?.code).toBe('control_plane.unreachable');
    actor.send({ type: 'ONLINE' });
    actor.send({ type: 'LOGIN' });
    expect(actor.getSnapshot().matches('signOutFailed')).toBe(true);

    actor.send({ type: 'RETRY' });
    await waitFor(actor, (snapshot) => snapshot.matches('unauthenticated'));
    expect(logout).toHaveBeenCalledTimes(2);
    expect(actor.getSnapshot().context.failure).toBeNull();
    actor.stop();
  });

  it('Matrix 注销抛出异常也会执行控制平面注销并报告失败', async () => {
    const logout = vi.fn<ControlPlaneGateway['logout']>().mockResolvedValue(ok(undefined));
    const runtime = dependencies({
      controlPlane: { logout },
      matrix: { logout: () => Promise.reject(new Error('测试适配器异常')) },
    });
    const actor = createActor(createSessionMachine(runtime.value)).start();
    await waitFor(actor, (snapshot) => snapshot.matches('ready'));

    actor.send({ type: 'LOGOUT' });
    const failed = await waitFor(actor, (snapshot) => snapshot.matches('signOutFailed'));

    expect(logout).toHaveBeenCalledOnce();
    expect(failed.context.failure?.code).toBe('session.cleanup_failed');
    actor.stop();
  });

  it('重新联网收到 401 时立即隔离旧身份、Matrix 连接和私有缓存', async () => {
    const readSession = vi
      .fn<ControlPlaneGateway['readSession']>()
      .mockResolvedValueOnce(ok(session))
      .mockResolvedValueOnce(
        err({
          boundary: 'control-plane',
          code: 'authentication.session_required',
          offline: false,
          retryable: false,
        }),
      );
    const disconnect = vi.fn();
    const clear = vi.fn();
    const runtime = dependencies({ controlPlane: { readSession }, matrix: { disconnect } });
    runtime.value.privateState.clear = clear;
    const actor = createActor(createSessionMachine(runtime.value)).start();
    await waitFor(actor, (snapshot) => snapshot.matches('ready'));
    clear.mockClear();
    disconnect.mockClear();

    actor.send({ type: 'OFFLINE' });
    actor.send({ type: 'ONLINE' });
    const ended = await waitFor(actor, (snapshot) => snapshot.matches('unauthenticated'));

    expect(ended.context.principal).toBeNull();
    expect(ended.context.connection).toBeNull();
    expect(ended.context.controlStatus).toBe('unauthenticated');
    expect(disconnect).toHaveBeenCalledOnce();
    expect(clear).toHaveBeenCalledOnce();
    actor.stop();
  });

  it('Matrix 不可用时保留独立的云端账户能力', async () => {
    const runtime = dependencies({
      matrix: {
        restore: () =>
          Promise.resolve(
            err({
              boundary: 'matrix',
              code: 'matrix.restore_failed',
              offline: false,
              retryable: true,
            }),
          ),
      },
    });
    const actor = createActor(createSessionMachine(runtime.value)).start();
    const degraded = await waitFor(actor, (snapshot) => snapshot.matches('degraded'));

    expect(degraded.context.controlStatus).toBe('ready');
    expect(degraded.context.principal).toEqual(session);
    expect(degraded.context.connection).toBeNull();
    actor.stop();
  });

  it('公开健康探针恢复后仍需重验身份，不能直接沿用旧账号', async () => {
    const readSession = vi
      .fn<ControlPlaneGateway['readSession']>()
      .mockResolvedValueOnce(ok(session))
      .mockResolvedValueOnce(
        err({
          boundary: 'control-plane',
          code: 'authentication.session_required',
          offline: false,
          retryable: false,
        }),
      );
    const runtime = dependencies({ controlPlane: { readSession } });
    const actor = createActor(createSessionMachine(runtime.value)).start();
    await waitFor(actor, (snapshot) => snapshot.matches('ready'));
    actor.send({
      type: 'CONTROL_DEGRADED',
      reachable: false,
      failure: {
        boundary: 'control-plane',
        code: 'control_plane.unreachable',
        offline: false,
        retryable: true,
      },
    });
    expect(actor.getSnapshot().context.controlStatus).toBe('unavailable');

    actor.send({ type: 'CONTROL_HEALTHY' });
    const ended = await waitFor(actor, (snapshot) => snapshot.matches('unauthenticated'));

    expect(readSession).toHaveBeenCalledTimes(2);
    expect(ended.context.principal).toBeNull();
    actor.stop();
  });

  it('恢复两条会话并在首次同步后回到原始深链', async () => {
    const runtime = dependencies();
    const actor = createActor(createSessionMachine(runtime.value)).start();

    const ready = await waitFor(actor, (snapshot) => snapshot.matches('ready'));

    expect(ready.context.principal).toEqual(session);
    expect(ready.context.connection?.userId).toBe(session.matrixUserId);
    expect(runtime.navigations).toEqual(['/lobby/public?directory=open']);
    expect(ready.context.resumePath).toBeNull();
    actor.stop();
  });

  it('没有控制面会话时给出可执行登录动作', async () => {
    const requestedPaths: string[] = [];
    const runtime = dependencies({
      controlPlane: {
        beginAuthentication: (path) => {
          requestedPaths.push(path);
          return Promise.resolve(ok({ kind: 'browser-navigation' }));
        },
        readSession: () =>
          Promise.resolve(
            err({
              boundary: 'control-plane',
              code: 'authentication.session_required',
              offline: false,
              retryable: false,
            }),
          ),
      },
    });
    const actor = createActor(createSessionMachine(runtime.value)).start();
    await waitFor(actor, (snapshot) => snapshot.matches('unauthenticated'));

    actor.send({ type: 'LOGIN' });
    await waitFor(actor, (snapshot) => snapshot.matches('awaitingBrowserNavigation'));

    expect(requestedPaths).toEqual(['/lobby/public?directory=open']);
    actor.stop();
  });

  it('桌面会话建立后重新读取服务端真相而不是伪造本地登录态', async () => {
    let reads = 0;
    const runtime = dependencies({
      controlPlane: {
        beginAuthentication: () => Promise.resolve(ok({ kind: 'session-established' })),
        readSession: () => {
          reads += 1;
          return reads === 1
            ? Promise.resolve(
                err({
                  boundary: 'control-plane',
                  code: 'authentication.session_required',
                  offline: false,
                  retryable: false,
                }),
              )
            : Promise.resolve(ok(session));
        },
      },
    });
    const actor = createActor(createSessionMachine(runtime.value)).start();
    await waitFor(actor, (snapshot) => snapshot.matches('unauthenticated'));

    actor.send({ type: 'LOGIN' });
    const ready = await waitFor(actor, (snapshot) => snapshot.matches('ready'));

    expect(reads).toBe(2);
    expect(ready.context.principal).toEqual(session);
    actor.stop();
  });

  it('桌面 Matrix 授权完成后直接恢复设备会话而不等待页面跳转', async () => {
    let restores = 0;
    const runtime = dependencies({
      matrix: {
        beginAuthentication: () => Promise.resolve(ok({ kind: 'session-established' })),
        restore: () => {
          restores += 1;
          return restores === 1
            ? Promise.resolve(ok({ kind: 'authentication-required' }))
            : Promise.resolve(ok({ connection: connection(), kind: 'connected' }));
        },
      },
    });
    const actor = createActor(createSessionMachine(runtime.value)).start();
    await waitFor(actor, (snapshot) => snapshot.matches('unauthenticated'));

    actor.send({ type: 'LOGIN' });
    const ready = await waitFor(actor, (snapshot) => snapshot.matches('ready'));

    expect(restores).toBe(2);
    expect(ready.context.connection?.userId).toBe(session.matrixUserId);
    actor.stop();
  });

  it('控制面健康恢复不会掩盖 Matrix 故障', async () => {
    const matrixFailure: SessionFailure = {
      boundary: 'matrix',
      code: 'matrix.restore_failed',
      offline: false,
      retryable: true,
    };
    const runtime = dependencies({
      matrix: { restore: () => Promise.resolve(err(matrixFailure)) },
    });
    const actor = createActor(createSessionMachine(runtime.value)).start();
    await waitFor(actor, (snapshot) => snapshot.matches('degraded'));

    actor.send({ type: 'CONTROL_HEALTHY' });

    expect(actor.getSnapshot().matches('degraded')).toBe(true);
    expect(actor.getSnapshot().context.failure).toEqual(matrixFailure);
    actor.stop();
  });
});
