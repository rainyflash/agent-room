import { describe, expect, it, vi } from 'vitest';

import {
  TauriDesktopRuntimeGateway,
  type TauriDesktopTransport,
} from '@/features/desktop/adapters/tauri-desktop-runtime-gateway';

const readySnapshot = {
  autostartEnabled: false,
  bridge: {
    authorization: null,
    session: null,
    lifecycle: {
      automaticRestartCount: 0,
      changedAtUnixMs: 1,
      diagnosticCode: null,
      lastFailureCode: null,
      lastExitCode: null,
      nextRetryAtUnixMs: null,
      ownership: 'managed',
      phase: 'ready',
    },
  },
  deepLink: null,
  manualHostConfiguration: {
    args: [],
    command: 'C:\\Agent Room\\agent-room-mcp.exe',
    serverName: 'agent_room',
    transport: 'stdio',
  },
  platform: 'windows',
  updatesConfigured: false,
  agentTarget: null,
} as const;

function transport(overrides: Partial<TauriDesktopTransport> = {}): TauriDesktopTransport {
  return {
    available: () => true,
    invoke: vi.fn().mockResolvedValue(readySnapshot),
    listen: vi.fn().mockResolvedValue(() => undefined),
    ...overrides,
  };
}

describe('Tauri 桌面运行时适配器', () => {
  it('浏览器模式拒绝原生命令而不尝试调用传输层', async () => {
    const invoke = vi.fn();
    const gateway = new TauriDesktopRuntimeGateway(transport({ available: () => false, invoke }));

    await expect(gateway.snapshot()).resolves.toEqual({
      error: { code: 'desktop.runtime.unavailable', retryable: false },
      ok: false,
    });
    expect(invoke).not.toHaveBeenCalled();
  });

  it('只接受经过闭合 schema 校验的命令响应', async () => {
    const gateway = new TauriDesktopRuntimeGateway(
      transport({ invoke: vi.fn().mockResolvedValue({ bridge: { phase: 'fake-ready' } }) }),
    );

    await expect(gateway.snapshot()).resolves.toEqual({
      error: { code: 'desktop.command.invalid_response', retryable: true },
      ok: false,
    });
  });

  it('默认 Agent 引导只传递语言并校验目标摘要', async () => {
    const invoke = vi.fn().mockResolvedValue({
      agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
      lobbyLanguage: 'zh-CN',
      publicLobbyCatalogId: '0198b601-77a2-7f41-b4f4-940f291951b8',
    });
    const gateway = new TauriDesktopRuntimeGateway(transport({ invoke }));

    const result = await gateway.bootstrapDefaultAgent('zh-CN');

    expect(result.ok).toBe(true);
    expect(invoke).toHaveBeenCalledWith('desktop_bootstrap_default_agent', {
      preferredLanguage: 'zh-CN',
    });
  });

  it('大厅命令只接受经过边界校验的身份与投影', async () => {
    const invoke = vi.fn().mockResolvedValue({
      agents: [],
      identity: {
        agent: {
          agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
          avatarUrl: null,
          displayName: 'Agent',
          matrixUserId: '@agent:matrix.test',
        },
        connectionState: 'ready',
        grantedCapabilities: [],
        instanceId: '0198b601-77a4-7bb8-83eb-a8fe68c97e44',
        matrixDeviceId: 'DEVICE',
        roomId: '!public:matrix.test',
      },
      messages: [],
      nextCursor: null,
      observedAtUnixMs: 1_200,
    });
    const gateway = new TauriDesktopRuntimeGateway(transport({ invoke }));

    await expect(gateway.readLobby()).resolves.toMatchObject({ ok: true });
    expect(invoke).toHaveBeenCalledWith('desktop_lobby_snapshot', {});
  });

  it('先订阅桌面登录结果再打开系统浏览器，并在完成后释放监听器', async () => {
    const listeners = new Map<string, (payload: unknown) => void>();
    const invoke = vi.fn().mockResolvedValue(undefined);
    const listen: TauriDesktopTransport['listen'] = (eventName, listener) => {
      listeners.set(eventName, listener);
      return Promise.resolve(() => {
        listeners.delete(eventName);
      });
    };
    const gateway = new TauriDesktopRuntimeGateway(transport({ invoke, listen }));

    const pending = gateway.beginHumanAuthentication('/workspace', 'sign-in');
    await vi.waitFor(() => {
      expect(listeners.has('desktop://human-session-changed')).toBe(true);
      expect(listeners.has('desktop://human-session-failed')).toBe(true);
    });
    expect(invoke).toHaveBeenCalledWith('desktop_begin_human_authentication', {
      intent: 'sign-in',
      returnPath: '/workspace',
    });
    listeners.get('desktop://human-session-changed')?.({
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
    });

    await expect(pending).resolves.toMatchObject({
      ok: true,
      value: { returnPath: '/workspace' },
    });
    expect(listeners.size).toBe(0);
  });

  it('Matrix 登录通过原生命令返回一次性授权而不订阅 WebView 导航', async () => {
    const invoke = vi.fn().mockResolvedValue({
      loginToken: 'single-use-token',
      returnPath: '/lobby/public',
    });
    const gateway = new TauriDesktopRuntimeGateway(transport({ invoke }));

    await expect(gateway.beginMatrixAuthentication('/lobby/public')).resolves.toEqual({
      ok: true,
      value: { loginToken: 'single-use-token', returnPath: '/lobby/public' },
    });
    expect(invoke).toHaveBeenCalledWith('desktop_begin_matrix_authentication', {
      returnPath: '/lobby/public',
    });
  });

  it.each([
    [
      '序列化失败对象',
      JSON.stringify({ code: 'desktop.matrix_session.loopback_bind_failed', retryable: true }),
      { code: 'desktop.matrix_session.loopback_bind_failed', retryable: true },
    ],
    [
      'Error 消息中的失败对象',
      new Error(
        JSON.stringify({ code: 'desktop.matrix_session.browser_open_failed', retryable: true }),
      ),
      { code: 'desktop.matrix_session.browser_open_failed', retryable: true },
    ],
    [
      'Tauri ACL 拒绝',
      'Command desktop_begin_matrix_authentication not allowed by ACL',
      { code: 'desktop.command.permission_denied', retryable: false },
    ],
  ])('保留%s的可操作诊断', async (_caseName, rejection, expected) => {
    const gateway = new TauriDesktopRuntimeGateway(
      transport({ invoke: vi.fn().mockRejectedValue(rejection) }),
    );

    await expect(gateway.beginMatrixAuthentication('/lobby/public')).resolves.toEqual({
      error: expected,
      ok: false,
    });
  });

  it('拒绝畸形或过大的原生命令错误而不把任意文本提升为故障码', async () => {
    const gateway = new TauriDesktopRuntimeGateway(
      transport({ invoke: vi.fn().mockRejectedValue(`{${'x'.repeat(1_024)}}`) }),
    );

    await expect(gateway.beginMatrixAuthentication('/lobby/public')).resolves.toEqual({
      error: { code: 'desktop.command.failed', retryable: true },
      ok: false,
    });
  });

  it('订阅两个白名单事件并在任一载荷失真时显式失败', async () => {
    const listeners = new Map<string, (payload: unknown) => void>();
    const onFailure = vi.fn();
    const onRuntimeChanged = vi.fn();
    const listen: TauriDesktopTransport['listen'] = (eventName, listener) => {
      listeners.set(eventName, listener);
      return Promise.resolve(() => {
        listeners.delete(eventName);
      });
    };
    const gateway = new TauriDesktopRuntimeGateway(
      transport({
        listen,
      }),
    );

    const result = await gateway.subscribe({
      onDeepLink: vi.fn(),
      onFailure,
      onRuntimeChanged,
    });
    expect(result.ok).toBe(true);
    listeners.get('desktop://runtime-changed')?.(readySnapshot.bridge);
    expect(onRuntimeChanged).toHaveBeenCalledWith(readySnapshot.bridge);
    listeners.get('desktop://runtime-changed')?.({ lifecycle: { phase: 'invented' } });
    expect(onFailure).toHaveBeenCalledWith({
      code: 'desktop.event.invalid_runtime',
      retryable: true,
    });
    if (result.ok) {
      result.value();
    }
    expect(listeners.size).toBe(0);
  });
});
