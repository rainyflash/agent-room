import { describe, expect, it, vi } from 'vitest';

import type { DesktopMatrixSessions } from './control-plane-desktop-matrix-sessions';
import type { MatrixIssuedSessionGateway } from './desktop-matrix-gateway';
import { DesktopMatrixGateway } from './desktop-matrix-gateway';
import { err, ok } from '@/shared/result';

const issuedLogin = {
  access_token: 'syt_access',
  device_id: 'DESKTOPDEV',
  refresh_token: 'syr_refresh',
  user_id: '@rainy:matrix.agent-room.test',
};

function matrixGateway(
  adoptIssuedSession: MatrixIssuedSessionGateway['adoptIssuedSession'],
): MatrixIssuedSessionGateway {
  return {
    disconnect: vi.fn(),
    beginAuthentication: () => Promise.resolve(ok({ kind: 'browser-navigation' })),
    adoptIssuedSession,
    logout: () => Promise.resolve(ok(undefined)),
    restore: () => Promise.resolve(ok({ kind: 'authentication-required' })),
  };
}

describe('DesktopMatrixGateway', () => {
  it.each(['disconnect', 'logout'] as const)('在 %s 后不要迟到的聊天登录', async (action) => {
    const issued = Promise.withResolvers<Awaited<ReturnType<DesktopMatrixSessions['issue']>>>();
    const adopt = vi.fn();
    const gateway = new DesktopMatrixGateway({
      matrix: matrixGateway(adopt),
      sessions: { issue: () => issued.promise },
    });
    const authentication = gateway.beginAuthentication('/lobby/public');
    await gateway[action]();
    issued.resolve(ok(issuedLogin));

    await expect(authentication).resolves.toMatchObject({
      ok: false,
      error: { code: 'matrix.session_superseded' },
    });
    expect(adopt).not.toHaveBeenCalled();
  });

  it('控制面发的聊天登录直接存下连上，不开浏览器', async () => {
    const adopt = vi.fn().mockResolvedValue(ok(undefined));
    const issue = vi.fn().mockResolvedValue(ok(issuedLogin));
    const gateway = new DesktopMatrixGateway({
      matrix: matrixGateway(adopt),
      sessions: { issue },
    });

    await expect(gateway.beginAuthentication('/lobby/public')).resolves.toEqual({
      ok: true,
      value: { kind: 'session-established' },
    });
    expect(issue).toHaveBeenCalledOnce();
    expect(adopt).toHaveBeenCalledWith(issuedLogin, '/lobby/public');
  });

  it('控制面没发出来时照实报，不去存', async () => {
    const adopt = vi.fn();
    const failure = {
      boundary: 'matrix',
      code: 'matrix_session.unavailable',
      offline: false,
      retryable: true,
    } as const;
    const gateway = new DesktopMatrixGateway({
      matrix: matrixGateway(adopt),
      sessions: { issue: () => Promise.resolve(err(failure)) },
    });

    await expect(gateway.beginAuthentication('/connect')).resolves.toEqual({
      error: failure,
      ok: false,
    });
    expect(adopt).not.toHaveBeenCalled();
  });
});
