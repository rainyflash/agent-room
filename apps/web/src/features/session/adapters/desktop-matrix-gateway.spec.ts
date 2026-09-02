import { describe, expect, it, vi } from 'vitest';

import type { MatrixAuthenticationSessionGateway } from './desktop-matrix-gateway';
import { DesktopMatrixGateway } from './desktop-matrix-gateway';
import { err, ok } from '@/shared/result';

function matrixGateway(
  exchangeAuthenticationGrant: MatrixAuthenticationSessionGateway['exchangeAuthenticationGrant'],
): MatrixAuthenticationSessionGateway {
  return {
    beginAuthentication: () => Promise.resolve(ok({ kind: 'browser-navigation' })),
    exchangeAuthenticationGrant,
    logout: () => Promise.resolve(ok(undefined)),
    restore: () => Promise.resolve(ok({ kind: 'authentication-required' })),
  };
}

describe('DesktopMatrixGateway', () => {
  it('把原生回环授权交换为已建立会话而不导航桌面 WebView', async () => {
    const exchange = vi.fn().mockResolvedValue(ok(undefined));
    const beginMatrixAuthentication = vi
      .fn()
      .mockResolvedValue(ok({ loginToken: 'single-use-token', returnPath: '/lobby/public' }));
    const gateway = new DesktopMatrixGateway({
      matrix: matrixGateway(exchange),
      runtime: { beginMatrixAuthentication },
    });

    await expect(gateway.beginAuthentication('/lobby/public')).resolves.toEqual({
      ok: true,
      value: { kind: 'session-established' },
    });
    expect(beginMatrixAuthentication).toHaveBeenCalledWith('/lobby/public');
    expect(exchange).toHaveBeenCalledWith('single-use-token', '/lobby/public');
  });

  it('原生回调失败时不尝试交换 Matrix 凭据', async () => {
    const exchange = vi.fn();
    const gateway = new DesktopMatrixGateway({
      matrix: matrixGateway(exchange),
      runtime: {
        beginMatrixAuthentication: () =>
          Promise.resolve(
            err({ code: 'desktop.matrix_session.loopback_timeout', retryable: true }),
          ),
      },
    });

    await expect(gateway.beginAuthentication('/connect')).resolves.toEqual({
      error: {
        boundary: 'matrix',
        code: 'desktop.matrix_session.loopback_timeout',
        offline: false,
        retryable: true,
      },
      ok: false,
    });
    expect(exchange).not.toHaveBeenCalled();
  });
});
