// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { AppProviders } from '@/app/app-providers';
import { createCloudRuntime } from '@/app/web-app-providers';
import { DesktopMatrixGateway } from '@/features/session/adapters/desktop-matrix-gateway';
import type {
  BridgeRuntime,
  DesktopRuntimeGateway,
  DesktopRuntimeSnapshot,
} from '@/features/desktop/domain/desktop-runtime';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

vi.mock('virtual:pwa-register/react', () => ({
  useRegisterSW: () => ({
    needRefresh: [false, vi.fn()],
    updateServiceWorker: vi.fn(),
  }),
}));

const config = {
  controlPlaneUrl: 'https://api.agent-room.test',
  matrixHomeserverUrl: 'https://matrix.agent-room.test',
  registrationMode: 'open-email' as const,
  windowsDownloadUrl: null,
};

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en-US']);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('应用组合根', () => {
  it('云端服务始终存在，本机 Runtime 仅作为同一个服务图中的可选能力', () => {
    const localRuntime = runtimeGateway(false);

    const runtime = createCloudRuntime(config, localRuntime);

    expect(runtime.services.localRuntime).toBe(localRuntime);
    expect(runtime.services.agentDirectory).toBeDefined();
    expect(runtime.services.session.controlPlane).toBe(runtime.services.controlPlane);
    expect('runtimeMode' in runtime.services).toBe(false);
  });

  it('没有本机 Runtime 时仍渲染云端产品入口', async () => {
    window.history.replaceState(null, '', '/');

    renderApplication(runtimeGateway(false));

    expect(
      await screen.findByRole('heading', {
        name: 'A shared room for agents that are actually working.',
      }),
    ).toBeInTheDocument();
    expect(screen.queryByText('Desktop runtime')).not.toBeInTheDocument();
  });

  it('检测到本机 Runtime 时增强同一套路由，不再切换到平行桌面产品', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response(null, { status: 204 }));
    window.history.replaceState(null, '', '/');

    renderApplication(runtimeGateway(true));

    expect(
      await screen.findByRole('heading', {
        name: 'A shared room for agents that are actually working.',
      }),
    ).toBeInTheDocument();
    expect(await screen.findByText('Desktop runtime')).toBeVisible();
    expect(screen.queryByText('Starting the local Agent runtime')).not.toBeInTheDocument();
  });

  it('桌面组合根只替换 Matrix 认证入口而保留同一云端服务图', () => {
    const runtime = createCloudRuntime(config, runtimeGateway(true));

    expect(runtime.services.session.matrix).toBeInstanceOf(DesktopMatrixGateway);
    expect(runtime.services.lobby).toBeDefined();
  });
});

function renderApplication(localRuntime: DesktopRuntimeGateway) {
  return render(
    <I18nextProvider i18n={i18n}>
      <AppProviders config={config} localRuntime={localRuntime} />
    </I18nextProvider>,
  );
}

function runtimeGateway(available: boolean): DesktopRuntimeGateway {
  const bridge: BridgeRuntime = {
    authorization: null,
    lifecycle: {
      automaticRestartCount: 0,
      changedAtUnixMs: 1,
      diagnosticCode: null,
      lastExitCode: null,
      lastFailureCode: null,
      nextRetryAtUnixMs: null,
      ownership: 'managed',
      phase: 'ready',
    },
    session: null,
  };
  const snapshot: DesktopRuntimeSnapshot = {
    agentTarget: null,
    autostartEnabled: false,
    bridge,
    deepLink: null,
    manualHostConfiguration: {
      args: [],
      command: 'C:\\Agent Room\\agent-room-mcp.exe',
      serverName: 'agent_room',
      transport: 'stdio',
    },
    platform: 'windows',
    updatesConfigured: false,
  };
  return {
    beginHumanAuthentication: () =>
      Promise.resolve(err({ code: 'desktop.test.unavailable', retryable: false })),
    beginMatrixAuthentication: () =>
      Promise.resolve(err({ code: 'desktop.test.unavailable', retryable: false })),
    bootstrapDefaultAgent: () =>
      Promise.resolve(err({ code: 'desktop.test.unavailable', retryable: false })),
    checkUpdate: () => Promise.resolve(err({ code: 'desktop.test.unavailable', retryable: false })),
    clearHumanSession: () => Promise.resolve(ok(undefined)),
    configureAgentRuntime: () =>
      Promise.resolve(err({ code: 'desktop.test.unavailable', retryable: false })),
    installUpdate: () =>
      Promise.resolve(err({ code: 'desktop.test.unavailable', retryable: false })),
    isAvailable: () => available,
    openAuthorization: () =>
      Promise.resolve(err({ code: 'desktop.test.unavailable', retryable: false })),
    readLobby: () => Promise.resolve(err({ code: 'desktop.test.unavailable', retryable: false })),
    retryBridge: () => Promise.resolve(ok(bridge)),
    setAutostart: (enabled) => Promise.resolve(ok(enabled)),
    snapshot: () => Promise.resolve(ok(snapshot)),
    subscribe: () => Promise.resolve(ok(() => undefined)),
  };
}
