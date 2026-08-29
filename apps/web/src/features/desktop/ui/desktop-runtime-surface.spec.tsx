// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { I18nextProvider } from 'react-i18next';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type {
  BridgeRuntime,
  DesktopRuntimeGateway,
  DesktopRuntimeSnapshot,
} from '@/features/desktop/domain/desktop-runtime';
import { DesktopRuntimeSurface } from '@/features/desktop/ui/desktop-runtime-surface';
import { DesktopRuntimeProvider } from '@/features/desktop/ui/desktop-runtime-provider';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { ok } from '@/shared/result';

const router = vi.hoisted(() => ({ navigate: vi.fn() }));

vi.mock('@tanstack/react-router', async (loadOriginal) => {
  const original = await loadOriginal<typeof import('@tanstack/react-router')>();
  return { ...original, useNavigate: () => router.navigate };
});

const authorizationRuntime: BridgeRuntime = {
  authorization: {
    expiresAtUnixMs: Date.now() + 600_000,
    promptId: 'authorization-7',
    userCode: 'ABCD-EFGH',
    verificationHost: 'identity.example',
  },
  session: null,
  lifecycle: {
    automaticRestartCount: 0,
    changedAtUnixMs: 1,
    diagnosticCode: null,
    lastFailureCode: null,
    lastExitCode: null,
    nextRetryAtUnixMs: null,
    ownership: 'managed',
    phase: 'authorization_required',
  },
};

function snapshot(bridge: BridgeRuntime, updatesConfigured = false): DesktopRuntimeSnapshot {
  return {
    autostartEnabled: false,
    bridge,
    deepLink: null,
    platform: 'windows',
    updatesConfigured,
    agentTarget: null,
  };
}

function gateway(bridge: BridgeRuntime, updatesConfigured = false) {
  const openAuthorization = vi.fn(async () => ok(undefined));
  const retryBridge = vi.fn(async () => ok(bridge));
  const checkUpdate = vi.fn(async (channel: 'stable' | 'testing') =>
    ok({
      available: true,
      channel,
      currentVersion: '0.1.0',
      rollback: false,
      sequence: 8,
      targetVersion: '0.2.0',
    }),
  );
  const installUpdate = vi.fn(async () => ok(undefined));
  const value: DesktopRuntimeGateway = {
    bootstrapDefaultAgent: async () =>
      ok({
        agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
        lobbyLanguage: 'en',
        publicLobbyCatalogId: '0198b601-77a2-7f41-b4f4-940f291951b8',
      }),
    checkUpdate,
    configureAgentRuntime: async (target) => ok(target),
    installUpdate,
    isAvailable: () => true,
    openAuthorization,
    readLobby: async () => {
      throw new Error('此测试不读取大厅。');
    },
    retryBridge,
    setAutostart: async (enabled) => ok(enabled),
    snapshot: async () => ok(snapshot(bridge, updatesConfigured)),
    subscribe: async () => ok(() => undefined),
  };
  return { checkUpdate, installUpdate, openAuthorization, retryBridge, value };
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en-US']);
});

afterEach(cleanup);

describe('桌面运行时界面', () => {
  it('只展示身份站点和一次性代码，并通过闭合命令打开完整地址', async () => {
    const runtime = gateway(authorizationRuntime);
    render(
      <I18nextProvider i18n={i18n}>
        <DesktopRuntimeProvider gateway={runtime.value}>
          <DesktopRuntimeSurface />
        </DesktopRuntimeProvider>
      </I18nextProvider>,
    );

    expect(await screen.findByText('Authorize this desktop')).toBeVisible();
    expect(screen.getByText('identity.example')).toBeVisible();
    expect(screen.getByText('ABCD-EFGH')).toBeVisible();
    expect(screen.queryByText(/https:\/\//u)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Open secure sign-in' }));
    await waitFor(() => {
      expect(runtime.openAuthorization).toHaveBeenCalledWith('authorization-7');
    });
  });

  it('崩溃预算耗尽后保持停机，只有明确按钮才触发重试', async () => {
    const halted: BridgeRuntime = {
      authorization: null,
      session: null,
      lifecycle: {
        ...authorizationRuntime.lifecycle,
        automaticRestartCount: 4,
        diagnosticCode: 'desktop.bridge.restart_budget_exhausted',
        lastFailureCode: 'bridge.identity.discovery_failed',
        phase: 'halted',
      },
    };
    const runtime = gateway(halted);
    render(
      <I18nextProvider i18n={i18n}>
        <DesktopRuntimeProvider gateway={runtime.value}>
          <DesktopRuntimeSurface />
        </DesktopRuntimeProvider>
      </I18nextProvider>,
    );

    expect(await screen.findByText('Automatic restart was stopped')).toBeVisible();
    expect(runtime.retryBridge).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Retry Bridge' }));
    await waitFor(() => {
      expect(runtime.retryBridge).toHaveBeenCalledTimes(1);
    });
  });

  it('只有签名更新已配置时才允许显式检查并安装同一序号', async () => {
    const ready: BridgeRuntime = {
      authorization: null,
      session: null,
      lifecycle: {
        ...authorizationRuntime.lifecycle,
        diagnosticCode: 'desktop.bridge.ready',
        phase: 'ready',
      },
    };
    const runtime = gateway(ready, true);
    render(
      <I18nextProvider i18n={i18n}>
        <DesktopRuntimeProvider gateway={runtime.value}>
          <DesktopRuntimeSurface />
        </DesktopRuntimeProvider>
      </I18nextProvider>,
    );

    fireEvent.click(await screen.findByRole('button', { name: /Desktop runtime/u }));
    fireEvent.click(screen.getByRole('button', { name: 'Check' }));
    await waitFor(() => {
      expect(runtime.checkUpdate).toHaveBeenCalledWith('stable');
      expect(screen.getByText('0.1.0 → 0.2.0')).toBeVisible();
    });
    fireEvent.click(screen.getByRole('button', { name: 'Install & restart' }));
    await waitFor(() => {
      expect(runtime.installUpdate).toHaveBeenCalledWith('stable', 8);
    });
  });
});
