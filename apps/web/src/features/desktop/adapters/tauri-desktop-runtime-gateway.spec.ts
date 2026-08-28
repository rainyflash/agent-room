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
