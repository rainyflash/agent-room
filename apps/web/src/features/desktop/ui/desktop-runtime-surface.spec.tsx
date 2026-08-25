// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { I18nextProvider } from 'react-i18next';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeAll, describe, expect, it, vi } from 'vitest';

import type {
  BridgeRuntime,
  DesktopRuntimeGateway,
  DesktopRuntimeSnapshot,
} from '@/features/desktop/domain/desktop-runtime';
import { DesktopRuntimeSurface } from '@/features/desktop/ui/desktop-runtime-surface';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { ok } from '@/shared/result';

vi.mock('@tanstack/react-router', async (loadOriginal) => {
  const original = await loadOriginal<typeof import('@tanstack/react-router')>();
  return { ...original, useNavigate: () => vi.fn() };
});

const authorizationRuntime: BridgeRuntime = {
  authorization: {
    expiresAtUnixMs: Date.now() + 600_000,
    promptId: 'authorization-7',
    userCode: 'ABCD-EFGH',
    verificationHost: 'identity.example',
  },
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

function snapshot(bridge: BridgeRuntime): DesktopRuntimeSnapshot {
  return {
    autostartEnabled: false,
    bridge,
    deepLink: null,
    platform: 'windows',
  };
}

function gateway(bridge: BridgeRuntime) {
  const openAuthorization = vi.fn(async () => ok(undefined));
  const retryBridge = vi.fn(async () => ok(bridge));
  const value: DesktopRuntimeGateway = {
    isAvailable: () => true,
    openAuthorization,
    retryBridge,
    setAutostart: async (enabled) => ok(enabled),
    snapshot: async () => ok(snapshot(bridge)),
    subscribe: async () => ok(() => undefined),
  };
  return { openAuthorization, retryBridge, value };
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en-US']);
});

describe('桌面运行时界面', () => {
  it('只展示身份站点和一次性代码，并通过闭合命令打开完整地址', async () => {
    const runtime = gateway(authorizationRuntime);
    render(
      <I18nextProvider i18n={i18n}>
        <DesktopRuntimeSurface gateway={runtime.value} />
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
        <DesktopRuntimeSurface gateway={runtime.value} />
      </I18nextProvider>,
    );

    expect(await screen.findByText('Automatic restart was stopped')).toBeVisible();
    expect(runtime.retryBridge).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Retry Bridge' }));
    await waitFor(() => {
      expect(runtime.retryBridge).toHaveBeenCalledTimes(1);
    });
  });
});
