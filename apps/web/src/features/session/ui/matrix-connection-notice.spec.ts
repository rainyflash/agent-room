import { describe, expect, it } from 'vitest';

import type { MatrixConnection, SessionFailure, WebSession } from '../domain/session';
import type { SessionContext } from '../domain/session-machine';
import { matrixConnectionNotice } from './matrix-connection-notice';

const principal: WebSession = {
  authenticatedAtUnixMs: 1_700_000_000_000,
  displayName: 'Local Developer',
  expiresAtUnixMs: 1_700_028_800_000,
  locale: 'zh-CN',
  matrixUserId: '@user-0123456789abcdef:matrix.agent-room.test',
  principalId: '018c251e-7b5a-7c7f-8a28-2de53f56a9a3',
  recentlyAuthenticated: true,
};

const connection: MatrixConnection = {
  deviceId: 'DESKTOPDEVICE',
  userId: principal.matrixUserId,
  disconnect: () => undefined,
  observe: () => () => undefined,
  waitUntilPrepared: () => Promise.reject(new Error('测试里不会用到')),
};

function failure(overrides: Partial<SessionFailure> = {}): SessionFailure {
  return {
    boundary: 'matrix',
    code: 'desktop.matrix_session.loopback_timeout',
    offline: false,
    retryable: true,
    ...overrides,
  };
}

function context(overrides: Partial<SessionContext> = {}): SessionContext {
  return {
    authenticationMode: 'automatic',
    authenticationTarget: 'matrix',
    connection: null,
    controlStatus: 'ready',
    failure: null,
    principal,
    resumePath: null,
    ...overrides,
  };
}

describe('消息没连上时怎么说', () => {
  it('没登录账户、消息已经连上时都不说', () => {
    expect(matrixConnectionNotice('degraded', context({ principal: null }), true)).toBeNull();
    expect(matrixConnectionNotice('ready', context({ connection }), true)).toBeNull();
  });

  it('正在恢复、同步、重连时只说正在连', () => {
    for (const state of ['booting', 'restoring', 'syncing', 'reconnecting'] as const) {
      expect(matrixConnectionNotice(state, context(), false)).toEqual({ kind: 'connecting' });
    }
  });

  it('桌面端等着浏览器里登录完时说出来；网页端是整页跳去登录，跳走前算正在连', () => {
    expect(matrixConnectionNotice('authenticating', context(), true)).toEqual({
      kind: 'browserSignIn',
    });
    expect(matrixConnectionNotice('awaitingBrowserNavigation', context(), false)).toEqual({
      kind: 'connecting',
    });
    expect(
      matrixConnectionNotice('authenticating', context({ authenticationTarget: 'control' }), true),
    ).toBeNull();
  });

  it('登录页等太久没回来：说消息没连上，给“重新连接”', () => {
    expect(matrixConnectionNotice('degraded', context({ failure: failure() }), true)).toEqual({
      kind: 'actionRequired',
      action: 'retry',
      actionKey: 'connection.action.reconnect',
      failureKey: 'connection.state.failure.connectionInterrupted',
    });
  });

  it('要重新登录时给“登录”，重的动作（退出登录）留给连接页', () => {
    expect(matrixConnectionNotice('unauthenticated', context(), true)).toMatchObject({
      kind: 'actionRequired',
      action: 'login',
      actionKey: 'connection.action.loginMatrix',
    });
    expect(
      matrixConnectionNotice(
        'degraded',
        context({ failure: failure({ code: 'matrix.identity_mismatch', retryable: false }) }),
        true,
      ),
    ).toEqual({
      kind: 'actionRequired',
      action: null,
      actionKey: null,
      failureKey: 'connection.state.failure.identityMismatch',
    });
  });

  it('只是控制面出错、消息还连着时，不算消息没连上', () => {
    expect(
      matrixConnectionNotice(
        'degraded',
        context({
          connection,
          failure: failure({ boundary: 'control-plane', code: 'control_plane.readiness_degraded' }),
        }),
        true,
      ),
    ).toBeNull();
  });
});
