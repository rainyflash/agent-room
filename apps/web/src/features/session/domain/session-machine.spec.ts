import { createActor, waitFor } from 'xstate';
import { describe, expect, it } from 'vitest';

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
  const value: SessionDependencies = { browser, controlPlane, matrix };
  return { navigations, value };
}

describe('Web 会话状态机', () => {
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
