import { describe, expect, it, vi } from 'vitest';

import { ControlPlaneDesktopMatrixSessions } from './control-plane-desktop-matrix-sessions';

const baseUrl = 'https://api.agent-room.test';

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    headers: { 'Content-Type': 'application/json' },
    status,
  });
}

describe('ControlPlaneDesktopMatrixSessions', () => {
  it('向控制面要聊天登录，原样交回 Matrix 登录的回答', async () => {
    const login = {
      access_token: 'syt_access',
      device_id: 'DESKTOPDEV',
      refresh_token: 'syr_refresh',
      user_id: '@rainy:matrix.agent-room.test',
    };
    const fetch = vi.fn().mockResolvedValue(json(200, login));
    const sessions = new ControlPlaneDesktopMatrixSessions({ baseUrl, fetch });

    await expect(sessions.issue()).resolves.toEqual({ ok: true, value: login });
    const [url, init] = fetch.mock.calls[0] as [URL, RequestInit];
    expect(String(url)).toBe(`${baseUrl}/auth/desktop/matrix-session`);
    expect(init.method).toBe('POST');
    expect(init.cache).toBe('no-store');
  });

  it.each([
    [403, 'authentication.desktop_session_required', false],
    [429, 'matrix_session.rate_limited', true],
    [503, 'matrix_session.unavailable', true],
  ])('控制面回 %i 时带上错误码，过一会能好的才说能重试', async (status, code, retryable) => {
    const fetch = vi
      .fn()
      .mockResolvedValue(json(status, { code, correlationId: 'corr-1', retryable }));
    const sessions = new ControlPlaneDesktopMatrixSessions({ baseUrl, fetch });

    await expect(sessions.issue()).resolves.toEqual({
      error: { boundary: 'matrix', code, correlationId: 'corr-1', offline: false, retryable },
      ok: false,
    });
  });

  it('连不上控制面时算可以重试', async () => {
    const fetch = vi.fn().mockRejectedValue(new TypeError('Failed to fetch'));
    const sessions = new ControlPlaneDesktopMatrixSessions({ baseUrl, fetch });

    await expect(sessions.issue()).resolves.toMatchObject({
      error: { code: 'matrix.desktop_session_unreachable', retryable: true },
      ok: false,
    });
  });
});
