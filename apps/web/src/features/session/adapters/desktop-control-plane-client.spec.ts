// @vitest-environment jsdom

import { describe, expect, it, vi } from 'vitest';

import type { DesktopRuntimeGateway } from '@/features/desktop/domain/desktop-runtime';
import { err, ok } from '@/shared/result';

import { ControlPlaneClient } from './control-plane-client';
import { DesktopControlPlaneClient } from './desktop-control-plane-client';

function runtime(
  overrides: Partial<DesktopRuntimeGateway> = {},
): DesktopRuntimeGateway {
  const unused = async (): Promise<never> => {
    throw new Error('此测试不应调用该桌面能力。');
  };
  return {
    beginHumanAuthentication: async () =>
      err({ code: 'desktop.test.unavailable', retryable: false }),
    bootstrapDefaultAgent: unused,
    checkUpdate: unused,
    clearHumanSession: async () => ok(undefined),
    configureAgentRuntime: unused,
    installUpdate: unused,
    isAvailable: () => true,
    openAuthorization: unused,
    readLobby: unused,
    retryBridge: unused,
    setAutostart: unused,
    snapshot: unused,
    subscribe: unused,
    ...overrides,
  };
}

describe('桌面 Control Plane 会话适配器', () => {
  it('系统浏览器回调完成后才导航，并把结果映射为已建立会话', async () => {
    const navigate = vi.fn();
    const localRuntime = runtime({
      beginHumanAuthentication: vi.fn(async () =>
        ok({
          returnPath: '/workspace',
          session: {
            authenticatedAtUnixMs: 1_700_000_000_000,
            displayName: 'Desktop User',
            expiresAtUnixMs: 1_700_028_800_000,
            locale: 'en',
            matrixUserId: '@desktop:matrix.test',
            principalId: '018c251e-7b5a-7c7f-8a28-2de53f56a9a3',
            recentlyAuthenticated: true,
          },
        }),
      ),
    });
    const client = new DesktopControlPlaneClient({
      controlPlane: new ControlPlaneClient({
        baseUrl: 'https://api.agent-room.test',
        fetch: vi.fn(),
      }),
      navigate,
      runtime: localRuntime,
    });

    await expect(client.beginAuthentication('/workspace', 'sign-in')).resolves.toEqual({
      ok: true,
      value: { kind: 'session-established' },
    });
    expect(localRuntime.beginHumanAuthentication).toHaveBeenCalledWith('/workspace', 'sign-in');
    expect(navigate).toHaveBeenCalledWith('/workspace');
  });

  it('远端注销失败时仍删除本机凭据，避免共享设备继续使用旧会话', async () => {
    const clearHumanSession = vi.fn(async () => ok(undefined));
    const client = new DesktopControlPlaneClient({
      controlPlane: new ControlPlaneClient({
        baseUrl: 'https://api.agent-room.test',
        fetch: vi.fn(() =>
          Promise.resolve(
            Response.json(
              { code: 'control_plane.unavailable', retryable: true },
              { status: 503 },
            ),
          ),
        ),
      }),
      runtime: runtime({ clearHumanSession }),
    });

    const result = await client.logout();

    expect(result.ok).toBe(false);
    expect(clearHumanSession).toHaveBeenCalledOnce();
  });
});
