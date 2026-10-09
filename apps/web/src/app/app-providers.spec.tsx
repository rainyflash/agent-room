// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { AppProviders } from '@/app/app-providers';
import { router } from '@/app/router';
import { ControlPlaneClient } from '@/features/session/adapters/control-plane-client';
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
  macosDownloadUrl: null,
};

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en-US']);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('应用组合根', () => {
  it('清理会话会取消私人 API 请求，同时保留正在执行的注销请求', async () => {
    const requests: { path: string; signal: AbortSignal | null | undefined }[] = [];
    const finish: (() => void)[] = [];
    vi.spyOn(globalThis, 'fetch').mockImplementation(
      (input, init) =>
        new Promise<Response>((resolve) => {
          const path = new URL(input instanceof Request ? input.url : String(input)).pathname;
          requests.push({ path, signal: init?.signal });
          finish.push(() => {
            resolve(
              path === '/auth/logout'
                ? new Response(null, { status: 204 })
                : Response.json(path === '/agents' ? { agents: [] } : { instances: [] }),
            );
          });
        }),
    );
    const runtime = createCloudRuntime(config, runtimeGateway(false));
    const pending = [
      runtime.services.agentDirectory.listOwnedAgents(),
      runtime.services.accessManagement.listAgentInstances(),
      runtime.services.controlPlane.logout(),
    ];
    try {
      runtime.services.session.privateState.clear();
      expect(requests.map(({ path, signal }) => ({ path, aborted: signal?.aborted }))).toEqual([
        { path: '/agents', aborted: true },
        { path: '/agent-instances', aborted: true },
        { path: '/auth/logout', aborted: false },
      ]);
    } finally {
      finish.forEach((complete) => {
        complete();
      });
      await Promise.all(pending);
    }
  });

  it('桌面端发往控制面的请求都交给原生层代发，清理会话照样取消私人请求', async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch');
    const requests: string[] = [];
    const answers: (() => void)[] = [];
    const sendControlPlaneRequest = vi.fn<DesktopRuntimeGateway['sendControlPlaneRequest']>(
      (request) =>
        new Promise((resolve) => {
          requests.push(`${request.method} ${request.path}`);
          answers.push(() => {
            resolve(
              ok({
                body: new TextEncoder().encode(
                  JSON.stringify({
                    checkedAtUnixMs: 1,
                    correlationId: '018c251e-7b5a-7c7f-8a28-2de53f56a9a3',
                    dependencies: [],
                    service: 'control-plane',
                    status: 'ready',
                    version: 'test',
                  }),
                ),
                headers: [['content-type', 'application/json']],
                status: 200,
              }),
            );
          });
        }),
    );
    const runtime = createCloudRuntime(config, {
      ...runtimeGateway(true),
      sendControlPlaneRequest,
    });

    const listed = runtime.services.agentDirectory.listOwnedAgents();
    const readiness = runtime.services.controlPlane.readReadiness();
    await vi.waitFor(() => {
      expect(requests).toEqual(['GET agents', 'GET health/ready']);
    });
    runtime.services.session.privateState.clear();
    answers.forEach((answer) => {
      answer();
    });

    await expect(listed).resolves.toMatchObject({ ok: false });
    await expect(readiness).resolves.toMatchObject({ ok: true });
    expect(fetchSpy).not.toHaveBeenCalled();
  });

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
        name: 'Bring any AI agent into your room with one message.',
      }),
    ).toBeInTheDocument();
    expect(screen.queryByText('Local agents')).not.toBeInTheDocument();
  });

  it('检测到本机 Runtime 时增强同一套路由，不再切换到平行桌面产品', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response(null, { status: 204 }));
    window.history.replaceState(null, '', '/');

    renderApplication(
      runtimeGateway(true, {
        expiresAtUnixMs: Date.now() + 600_000,
        promptId: 'authorization-1',
        userCode: 'ABCD-EFGH',
        verificationHost: 'identity.example',
      }),
    );

    expect(
      await screen.findByRole('heading', {
        name: 'Bring any AI agent into your room with one message.',
      }),
    ).toBeInTheDocument();
    // 同一个首页，桌面端多一条“允许这台电脑接入”的提示。
    expect(await screen.findByText('Allow this computer to connect your agents')).toBeVisible();
    expect(screen.queryByText('Starting the local Agent runtime')).not.toBeInTheDocument();
  });

  it('桌面组合根保留原生认证入口、自动防循环与同一云端服务图', async () => {
    vi.spyOn(DesktopMatrixGateway.prototype, 'restore').mockResolvedValue(
      ok({ kind: 'authentication-required' }),
    );
    const nativeAuthentication = vi
      .spyOn(DesktopMatrixGateway.prototype, 'beginAuthentication')
      .mockResolvedValue(ok({ kind: 'session-established' }));
    const runtime = createCloudRuntime(config, runtimeGateway(true));
    await runtime.services.session.matrix.restore('@composition:matrix.agent-room.test');
    await expect(
      runtime.services.session.matrix.beginAuthentication('/rooms', 'automatic'),
    ).resolves.toEqual(ok({ kind: 'session-established' }));
    await expect(
      runtime.services.session.matrix.beginAuthentication('/rooms', 'automatic'),
    ).resolves.toMatchObject({ ok: false, error: { code: 'matrix.authentication_interrupted' } });
    expect(nativeAuthentication).toHaveBeenCalledExactlyOnceWith('/rooms');
    window.sessionStorage.clear();
    expect(runtime.services.lobby).toBeDefined();
  });

  it('真实路由的“我的 Agent”页共享登录状态：这台电脑一节和接入对话框不会丢失 SessionProvider', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response(null, { status: 204 }));
    vi.spyOn(ControlPlaneClient.prototype, 'readSession').mockResolvedValue(
      ok({
        principalId: '018c251e-7b5a-7c7f-8a28-2de53f56a9a3',
        matrixUserId: '@ada:matrix.agent-room.test',
        displayName: 'Ada',
        locale: 'en',
        authenticatedAtUnixMs: 1,
        expiresAtUnixMs: Date.now() + 60_000,
        recentlyAuthenticated: true,
      }),
    );
    vi.spyOn(DesktopMatrixGateway.prototype, 'restore').mockResolvedValue(
      ok({
        kind: 'connected',
        connection: {
          deviceId: 'TEST',
          userId: '@ada:matrix.agent-room.test',
          disconnect: () => undefined,
          observe: () => () => undefined,
          waitUntilPrepared: () => Promise.resolve(ok(undefined)),
        },
      }),
    );
    renderApplication({
      ...runtimeGateway(true),
      listReceivers: () => Promise.resolve(ok([])),
      readHostSessions: () => Promise.resolve(ok([])),
    });
    await act(() => router.navigate({ search: {}, to: '/workspace' }));
    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'Background replies' })).toBeVisible();
    });
    expect(screen.getByRole('heading', { name: 'This computer' })).toBeVisible();
    const [invite] = screen.getAllByRole('button', { name: 'Bring an agent' });
    if (invite === undefined) throw new Error('invite button missing');
    fireEvent.click(invite);
    // 接入方式默认是网络接入；本机接入要先选命令行。
    fireEvent.click(await screen.findByRole('radio', { name: 'Command line' }));
    // Agent 自己起名：没有名字输入框，一段话一个复制按钮。
    expect(screen.queryByRole('textbox', { name: 'Agent name' })).toBeNull();
    expect(await screen.findByRole('button', { name: 'Copy message' })).toBeEnabled();
    expect(screen.queryByText('SessionProvider is missing.')).toBeNull();
  });
});

function renderApplication(localRuntime: DesktopRuntimeGateway) {
  return render(
    <I18nextProvider i18n={i18n}>
      <AppProviders config={config} localRuntime={localRuntime} />
    </I18nextProvider>,
  );
}

function runtimeGateway(
  available: boolean,
  authorization: BridgeRuntime['authorization'] = null,
): DesktopRuntimeGateway {
  const bridge: BridgeRuntime = {
    authorization,
    deviceReauthorizationAvailable: false,
    lifecycle: {
      automaticRestartCount: 0,
      changedAtUnixMs: 1,
      diagnosticCode: null,
      lastExitCode: null,
      lastFailureCode: null,
      nextRetryAtUnixMs: null,
      ownership: 'managed',
      phase: authorization === null ? 'ready' : 'authorization_required',
    },
    session: null,
  };
  const snapshot: DesktopRuntimeSnapshot = {
    agentTarget: null,
    autostartEnabled: false,
    bridge,
    deepLink: null,
    cliConfiguration: { command: 'C:\\Agent Room\\agent-room.exe', args: [] },
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
    restoreHumanSession: () => Promise.resolve(ok(true)),
    sendControlPlaneRequest: () =>
      Promise.resolve(ok({ status: 204, headers: [], body: new Uint8Array() })),
    configureAgentRuntime: () =>
      Promise.resolve(err({ code: 'desktop.test.unavailable', retryable: false })),
    installUpdate: () =>
      Promise.resolve(err({ code: 'desktop.test.unavailable', retryable: false })),
    isAvailable: () => available,
    openAuthorization: () =>
      Promise.resolve(err({ code: 'desktop.test.unavailable', retryable: false })),
    retryBridge: () => Promise.resolve(ok(bridge)),
    reauthorizeBridge: () => Promise.resolve(ok(bridge)),
    setAutostart: (enabled) => Promise.resolve(ok(enabled)),
    snapshot: () => Promise.resolve(ok(snapshot)),
    subscribe: () => Promise.resolve(ok(() => undefined)),
  };
}
