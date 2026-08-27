// @vitest-environment jsdom

import { describe, expect, it, vi } from 'vitest';

import { ControlPlaneClient } from './control-plane-client';

const validSession = {
  authenticatedAtUnixMs: 1_700_000_000_000,
  displayName: 'Local Developer',
  expiresAtUnixMs: 1_700_028_800_000,
  locale: 'en',
  matrixUserId: '@user-0123456789abcdef:matrix.agent-room.test',
  principalId: '018c251e-7b5a-7c7f-8a28-2de53f56a9a3',
  recentlyAuthenticated: true,
};

describe('ControlPlaneClient', () => {
  it('把注册意图作为受约束参数发送给身份入口', () => {
    const navigate = vi.fn();
    const client = new ControlPlaneClient({
      baseUrl: 'https://api.agent-room.test',
      fetch: vi.fn(),
      navigate,
    });

    client.beginAuthentication('/connect', 'register');

    expect(navigate).toHaveBeenCalledWith(
      'https://api.agent-room.test/auth/oidc/start?returnTo=%2Fconnect&importDisplayName=true&importLocale=true&intent=register',
    );
  });

  it('把 401 精确映射为缺少会话而不是通用网络错误', async () => {
    const client = new ControlPlaneClient({
      baseUrl: 'https://api.agent-room.test',
      fetch: vi.fn(() => Promise.resolve(new Response(null, { status: 401 }))),
    });

    const result = await client.readSession();

    expect(result).toEqual({
      error: {
        boundary: 'control-plane',
        code: 'authentication.session_required',
        offline: false,
        retryable: false,
      },
      ok: false,
    });
  });

  it('拒绝结构不完整的成功响应', async () => {
    const client = new ControlPlaneClient({
      baseUrl: 'https://api.agent-room.test',
      fetch: vi.fn(() =>
        Promise.resolve(Response.json({ ...validSession, matrixUserId: 'not-a-matrix-id' })),
      ),
    });

    const result = await client.readSession();

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.error.code).toBe('control_plane.invalid_session_response');
    }
  });

  it('保留 503 就绪报告中的精确降级事实', async () => {
    const report = {
      checkedAtUnixMs: 1_700_000_000_000,
      correlationId: '018c251e-7b5a-7c7f-8a28-2de53f56a9a3',
      dependencies: [
        {
          failure: 'timeout',
          latencyMs: 2_000,
          name: 'matrix',
          status: 'unavailable',
        },
      ],
      service: 'agent-room-control-plane',
      status: 'degraded',
      version: '0.1.0',
    };
    const client = new ControlPlaneClient({
      baseUrl: 'https://api.agent-room.test',
      fetch: vi.fn(() => Promise.resolve(Response.json(report, { status: 503 }))),
    });

    const result = await client.readReadiness();

    expect(result).toEqual({ ok: true, value: report });
  });
});
