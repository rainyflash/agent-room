// @vitest-environment jsdom

import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type {
  DesktopAgentTarget,
  DesktopRuntimeGateway,
  DesktopRuntimeSnapshot,
} from '@/features/desktop/domain/desktop-runtime';
import { DesktopConnectionPage } from '@/features/desktop/ui/desktop-connection-page';
import { DesktopRuntimeProvider } from '@/features/desktop/ui/desktop-runtime-provider';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { ok } from '@/shared/result';

const target: DesktopAgentTarget = {
  agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
  lobbyLanguage: 'en',
  publicLobbyCatalogId: '0198b601-77a2-7f41-b4f4-940f291951b8',
};

const authorizedSnapshot: DesktopRuntimeSnapshot = {
  agentTarget: null,
  autostartEnabled: false,
  bridge: {
    authorization: null,
    lifecycle: {
      automaticRestartCount: 0,
      changedAtUnixMs: 1,
      diagnosticCode: null,
      lastExitCode: null,
      lastFailureCode: null,
      nextRetryAtUnixMs: null,
      ownership: 'managed',
      phase: 'authorized',
    },
    session: null,
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
};

const router = vi.hoisted(() => ({ navigate: vi.fn() }));

vi.mock('@tanstack/react-router', async (loadOriginal) => {
  const original = await loadOriginal<typeof import('@tanstack/react-router')>();
  return { ...original, useNavigate: () => router.navigate };
});

function gateway(bootstrapDefaultAgent: DesktopRuntimeGateway['bootstrapDefaultAgent']) {
  const value: DesktopRuntimeGateway = {
    bootstrapDefaultAgent,
    checkUpdate: async () =>
      ok({
        available: false,
        channel: 'stable',
        currentVersion: '0.1.0',
        rollback: false,
        sequence: 1,
        targetVersion: '0.1.0',
      }),
    configureAgentRuntime: async (nextTarget) => ok(nextTarget),
    installUpdate: async () => ok(undefined),
    isAvailable: () => true,
    openAuthorization: async () => ok(undefined),
    readLobby: async () => {
      throw new Error('此测试不读取大厅。');
    },
    retryBridge: async () => ok(authorizedSnapshot.bridge),
    setAutostart: async (enabled) => ok(enabled),
    snapshot: async () => ok(authorizedSnapshot),
    subscribe: async () => ok(() => undefined),
  };
  return value;
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en-US']);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('桌面设备连接页', () => {
  it('设备已授权时自动且仅一次请求幂等默认 Agent', async () => {
    const bootstrap = vi.fn(async () => ok(target));
    render(
      <I18nextProvider i18n={i18n}>
        <DesktopRuntimeProvider gateway={gateway(bootstrap)}>
          <DesktopConnectionPage />
        </DesktopRuntimeProvider>
      </I18nextProvider>,
    );

    expect(
      await screen.findByRole('heading', { name: 'Preparing your first Agent' }),
    ).not.toBeNull();
    await waitFor(() => {
      expect(bootstrap).toHaveBeenCalledTimes(1);
      expect(bootstrap).toHaveBeenCalledWith('en');
    });
  });

  it('Bridge 与默认 Agent 就绪后自动进入真实大厅路由', async () => {
    const readySnapshot: DesktopRuntimeSnapshot = {
      ...authorizedSnapshot,
      agentTarget: target,
      bridge: {
        authorization: null,
        lifecycle: { ...authorizedSnapshot.bridge.lifecycle, phase: 'ready' },
        session: {
          agentId: target.agentId,
          instanceId: '0198b601-77a4-7bb8-83eb-a8fe68c97e44',
          matrixRoomId: '!public:matrix.test',
        },
      },
    };
    const runtime = gateway(vi.fn(async () => ok(target)));
    runtime.snapshot = async () => ok(readySnapshot);

    render(
      <I18nextProvider i18n={i18n}>
        <DesktopRuntimeProvider gateway={runtime}>
          <DesktopConnectionPage />
        </DesktopRuntimeProvider>
      </I18nextProvider>,
    );

    await waitFor(() => {
      expect(router.navigate).toHaveBeenCalledWith({
        params: {
          catalogId: target.publicLobbyCatalogId,
          roomId: '!public:matrix.test',
        },
        replace: true,
        search: {},
        to: '/lobby/$catalogId/instance/$roomId',
      });
    });
  });

  it('自动重启期间优先显示 Bridge 原始失败代码', async () => {
    const retrySnapshot: DesktopRuntimeSnapshot = {
      ...authorizedSnapshot,
      bridge: {
        authorization: null,
        lifecycle: {
          ...authorizedSnapshot.bridge.lifecycle,
          automaticRestartCount: 1,
          diagnosticCode: 'desktop.bridge.process_exited',
          lastExitCode: 1,
          lastFailureCode: 'bridge.matrix_store_unavailable',
          nextRetryAtUnixMs: 2_000,
          phase: 'retry_scheduled',
        },
        session: null,
      },
    };
    const runtime = gateway(vi.fn(async () => ok(target)));
    runtime.snapshot = async () => ok(retrySnapshot);

    render(
      <I18nextProvider i18n={i18n}>
        <DesktopRuntimeProvider gateway={runtime}>
          <DesktopConnectionPage />
        </DesktopRuntimeProvider>
      </I18nextProvider>,
    );

    expect(await screen.findByText('bridge.matrix_store_unavailable')).not.toBeNull();
    expect(screen.queryByText('desktop.bridge.process_exited')).toBeNull();
  });
});
