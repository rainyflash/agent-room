// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { I18nextProvider } from 'react-i18next';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type {
  BridgeRuntime,
  DesktopRuntimeGateway,
  DesktopRuntimeSnapshot,
} from '@/features/desktop/domain/desktop-runtime';
import { DesktopRuntimeSurface } from '@/features/desktop/ui/desktop-runtime-surface';
import { DesktopRuntimeProvider } from '@/features/desktop/ui/desktop-runtime-provider';
import { LocalConnectionNotice } from '@/features/desktop/ui/local-connection-notice';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';
import { ApplicationAboutPage } from '@/features/updates/ui/application-about-page';
import { RouterTestProvider } from '@/test/router-test-provider';

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
  deviceReauthorizationAvailable: false,
};

function snapshot(
  bridge: BridgeRuntime,
  updatesConfigured = false,
  currentVersion?: string,
): DesktopRuntimeSnapshot {
  return {
    autostartEnabled: false,
    bridge,
    ...(currentVersion === undefined ? {} : { currentVersion }),
    deepLink: null,
    cliConfiguration: { command: 'C:\\Agent Room\\agent-room.exe', args: [] },
    manualHostConfiguration: {
      args: [],
      command: 'C:\\Agent Room\\agent-room-mcp.exe',
      serverName: 'agent_room',
      transport: 'stdio',
    },
    platform: 'windows',
    updatesConfigured,
    agentTarget: null,
  };
}

function gateway(bridge: BridgeRuntime, updatesConfigured = false, currentVersion?: string) {
  const openAuthorization = vi.fn(() => Promise.resolve(ok(undefined)));
  const retryBridge = vi.fn(() => Promise.resolve(ok(bridge)));
  const reauthorizeBridge = vi.fn(() =>
    Promise.resolve(
      ok({
        ...bridge,
        deviceReauthorizationAvailable: false,
        lifecycle: { ...bridge.lifecycle, phase: 'starting' as const },
      }),
    ),
  );
  const checkUpdate = vi.fn((channel: 'stable' | 'testing') =>
    Promise.resolve(
      ok({
        available: true,
        channel,
        currentVersion: '0.1.0',
        rollback: false,
        sequence: 8,
        targetVersion: '0.2.0',
      }),
    ),
  );
  const installUpdate = vi.fn(() => Promise.resolve(ok(undefined)));
  const openLogs = vi.fn(() => Promise.resolve(ok(undefined)));
  const value: DesktopRuntimeGateway = {
    beginHumanAuthentication: () =>
      Promise.resolve(err({ code: 'desktop.test.unavailable', retryable: false })),
    beginMatrixAuthentication: () =>
      Promise.resolve(err({ code: 'desktop.test.unavailable', retryable: false })),
    bootstrapDefaultAgent: () =>
      Promise.resolve(
        ok({
          agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
          lobbyLanguage: 'en',
          publicLobbyCatalogId: '0198b601-77a2-7f41-b4f4-940f291951b8',
        }),
      ),
    checkUpdate,
    clearHumanSession: () => Promise.resolve(ok(undefined)),
    restoreHumanSession: () => Promise.resolve(ok(true)),
    configureAgentRuntime: (target) => Promise.resolve(ok(target)),
    installUpdate,
    isAvailable: () => true,
    openAuthorization,
    openLogs,
    readLobby: () => {
      return Promise.reject(new Error('此测试不读取大厅。'));
    },
    retryBridge,
    reauthorizeBridge,
    setAutostart: (enabled) => Promise.resolve(ok(enabled)),
    snapshot: () => Promise.resolve(ok(snapshot(bridge, updatesConfigured, currentVersion))),
    subscribe: () => Promise.resolve(ok(() => undefined)),
  };
  return {
    checkUpdate,
    installUpdate,
    openAuthorization,
    openLogs,
    reauthorizeBridge,
    retryBridge,
    value,
  };
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en-US']);
});

afterEach(cleanup);

describe('桌面运行时界面', () => {
  it('未授权本机 Agent 也可在关于页检查升级，异常后按钮恢复并允许重试', async () => {
    const runtime = gateway(authorizationRuntime, true);
    runtime.checkUpdate.mockRejectedValueOnce(new Error('transport unavailable'));
    render(
      <RouterTestProvider>
        <I18nextProvider i18n={i18n}>
          <DesktopRuntimeProvider gateway={runtime.value}>
            <ApplicationAboutPage />
          </DesktopRuntimeProvider>
        </I18nextProvider>
      </RouterTestProvider>,
    );
    const check = await screen.findByRole('button', { name: 'Check' });
    fireEvent.click(check);
    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent('The update did not complete');
    });
    expect(check).toBeEnabled();
    fireEvent.click(check);
    const install = await screen.findByRole('button', { name: 'Install and restart' });
    fireEvent.click(install);
    await waitFor(() => {
      expect(runtime.installUpdate).toHaveBeenCalledWith('testing', 8);
    });
    expect(runtime.openAuthorization).not.toHaveBeenCalled();
  });
  it('启动后自动按本版所属渠道查一次更新，折叠标题显示可安装的新版本', async () => {
    const ready: BridgeRuntime = {
      authorization: null,
      session: null,
      lifecycle: {
        ...authorizationRuntime.lifecycle,
        diagnosticCode: 'desktop.bridge.ready',
        phase: 'ready',
      },
      deviceReauthorizationAvailable: false,
    };
    // 预发行版来自测试渠道；没人会主动点“检查”，所以启动后就查一次。
    const runtime = gateway(ready, true, '0.1.0-alpha.47');
    render(
      <I18nextProvider i18n={i18n}>
        <DesktopRuntimeProvider gateway={runtime.value}>
          <DesktopRuntimeSurface />
        </DesktopRuntimeProvider>
      </I18nextProvider>,
    );

    await waitFor(() => {
      expect(runtime.checkUpdate).toHaveBeenCalledWith('testing');
    });
    expect(runtime.checkUpdate).toHaveBeenCalledTimes(1);
    const trigger = await screen.findByRole('button', { name: /Update 0\.2\.0 ready to install/u });
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(trigger);
    fireEvent.click(await screen.findByRole('button', { name: 'Install & restart' }));
    await waitFor(() => {
      expect(runtime.installUpdate).toHaveBeenCalledWith('testing', 8);
    });
  });
  it('Bridge 停机时标题仍报告停机，但展开后照样能安装更新', async () => {
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
      deviceReauthorizationAvailable: true,
    };
    const runtime = gateway(halted, true, '0.1.0-alpha.47');
    render(
      <I18nextProvider i18n={i18n}>
        <DesktopRuntimeProvider gateway={runtime.value}>
          <DesktopRuntimeSurface />
        </DesktopRuntimeProvider>
      </I18nextProvider>,
    );

    await waitFor(() => {
      expect(runtime.checkUpdate).toHaveBeenCalledWith('testing');
    });
    // 需要处理的状态优先于新版本提示。
    const trigger = await screen.findByRole('button', { name: /Local agents/u });
    expect(screen.queryByText(/ready to install/u)).not.toBeInTheDocument();
    fireEvent.click(trigger);
    // 升级往往正是修复停机的办法，所以停机时更新区仍在。
    fireEvent.click(await screen.findByRole('button', { name: 'Install & restart' }));
    await waitFor(() => {
      expect(runtime.installUpdate).toHaveBeenCalledWith('testing', 8);
    });
  });
  it('展开后能打开日志文件夹，出问题时有东西可发', async () => {
    const runtime = gateway(authorizationRuntime);
    render(
      <I18nextProvider i18n={i18n}>
        <DesktopRuntimeProvider gateway={runtime.value}>
          <DesktopRuntimeSurface />
        </DesktopRuntimeProvider>
      </I18nextProvider>,
    );

    fireEvent.click(await screen.findByRole('button', { name: /Local agents/u }));
    await waitFor(() => {
      expect(screen.getByText(/Send desktop\.log and bridge\.log/u)).toBeVisible();
    });
    fireEvent.click(screen.getByRole('button', { name: 'Open log folder' }));
    await waitFor(() => {
      expect(runtime.openLogs).toHaveBeenCalledTimes(1);
    });
  });
  it('只展示身份站点和一次性代码，并通过闭合命令打开完整地址', async () => {
    const runtime = gateway(authorizationRuntime);
    render(
      <I18nextProvider i18n={i18n}>
        <DesktopRuntimeProvider gateway={runtime.value}>
          <DesktopRuntimeSurface />
        </DesktopRuntimeProvider>
      </I18nextProvider>,
    );

    fireEvent.click(await screen.findByRole('button', { name: /Local agents/u }));
    await waitFor(() => {
      expect(screen.getByText('Authorize local agents')).toBeVisible();
    });
    expect(screen.getByText('identity.example')).toBeVisible();
    expect(screen.getByText('ABCD-EFGH')).toBeVisible();
    expect(screen.queryByText(/https:\/\//u)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Open secure sign-in' }));
    await waitFor(() => {
      expect(runtime.openAuthorization).toHaveBeenCalledWith('authorization-7');
    });
  });

  it('崩溃预算耗尽后只标记注意状态，显式展开后才允许重试', async () => {
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
      deviceReauthorizationAvailable: true,
    };
    const runtime = gateway(halted);
    render(
      <I18nextProvider i18n={i18n}>
        <DesktopRuntimeProvider gateway={runtime.value}>
          <DesktopRuntimeSurface />
        </DesktopRuntimeProvider>
      </I18nextProvider>,
    );

    const trigger = await screen.findByRole('button', { name: /Local agents/u });
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    expect(trigger.closest('aside')).toHaveAttribute('data-attention', 'true');
    expect(screen.queryByText('Automatic restart was stopped')).not.toBeInTheDocument();
    expect(runtime.retryBridge).not.toHaveBeenCalled();
    fireEvent.click(trigger);
    await waitFor(() => {
      expect(screen.getByText('Automatic restart was stopped')).toBeVisible();
    });
    fireEvent.click(screen.getByRole('button', { name: 'Reconnect local agents' }));
    await waitFor(() => {
      expect(runtime.retryBridge).toHaveBeenCalledTimes(1);
    });
  });

  it('停机视图提供重新授权这台电脑，并只通过闭合命令清除本机凭据', async () => {
    const halted: BridgeRuntime = {
      authorization: null,
      session: null,
      lifecycle: {
        ...authorizationRuntime.lifecycle,
        automaticRestartCount: 3,
        diagnosticCode: 'desktop.bridge.restart_budget_exhausted',
        lastExitCode: 1,
        lastFailureCode: 'bridge.refresh_outcome_unknown',
        phase: 'halted',
      },
      deviceReauthorizationAvailable: true,
    };
    const runtime = gateway(halted);
    render(
      <I18nextProvider i18n={i18n}>
        <DesktopRuntimeProvider gateway={runtime.value}>
          <DesktopRuntimeSurface />
        </DesktopRuntimeProvider>
      </I18nextProvider>,
    );

    fireEvent.click(await screen.findByRole('button', { name: /Local agents/u }));
    await waitFor(() => {
      expect(screen.getByText('Automatic restart was stopped')).toBeVisible();
    });
    expect(
      screen.getByText(
        'If reconnecting doesn’t help, re-authorize this computer. It clears the device credential saved here and asks for a new one-time code.',
      ),
    ).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Re-authorize this computer' }));
    await waitFor(() => {
      expect(runtime.reauthorizeBridge).toHaveBeenCalledTimes(1);
    });
    expect(runtime.retryBridge).not.toHaveBeenCalled();
    await waitFor(() => {
      expect(
        screen.queryByRole('button', { name: 'Re-authorize this computer' }),
      ).not.toBeInTheDocument();
    });
  });

  it('连不上服务器时说明原因和下次尝试时间，不当作崩溃，也允许立即重试', async () => {
    const nextRetryAtUnixMs = Date.UTC(2026, 8, 18, 17, 8, 57);
    const runtime = gateway({
      authorization: null,
      session: null,
      lifecycle: {
        ...authorizationRuntime.lifecycle,
        diagnosticCode: 'desktop.bridge.server_unreachable',
        lastFailureCode: 'bridge.identity_provider_unavailable',
        nextRetryAtUnixMs,
        phase: 'retry_scheduled',
      },
      deviceReauthorizationAvailable: false,
    });
    const unreachable =
      'Can’t reach Agent Room right now. Retrying automatically; you don’t need to restart the app.';
    const nextAttempt = `Next attempt at ${new Intl.DateTimeFormat(i18n.resolvedLanguage, {
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    }).format(new Date(nextRetryAtUnixMs))}`;
    render(
      <I18nextProvider i18n={i18n}>
        <DesktopRuntimeProvider gateway={runtime.value}>
          <LocalConnectionNotice />
          <DesktopRuntimeSurface />
        </DesktopRuntimeProvider>
      </I18nextProvider>,
    );

    expect(await screen.findByText(unreachable)).toBeVisible();
    expect(screen.getByText(nextAttempt)).toBeVisible();
    expect(
      screen.queryByText(
        'The agent connection service stopped unexpectedly. Restarting automatically.',
      ),
    ).not.toBeInTheDocument();
    expect(screen.getByText('bridge.identity_provider_unavailable')).not.toBeVisible();
    fireEvent.click(screen.getByText('Connection and identity details'));
    expect(screen.getByText('bridge.identity_provider_unavailable')).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Retry connection' }));
    await waitFor(() => {
      expect(runtime.retryBridge).toHaveBeenCalledTimes(1);
    });

    const trigger = screen.getByRole('button', { name: /Local agents/u });
    expect(within(trigger).getByText('Can’t reach Agent Room')).toBeVisible();
    expect(trigger.closest('aside')).toHaveAttribute('data-attention', 'false');
    fireEvent.click(trigger);
    await waitFor(() => {
      expect(screen.getAllByText(unreachable)).toHaveLength(2);
    });
    expect(screen.queryByText('Automatic restart was stopped')).not.toBeInTheDocument();
  });

  it('为底部主操作栏声明独立避让位置', async () => {
    const runtime = gateway(authorizationRuntime);
    render(
      <I18nextProvider i18n={i18n}>
        <DesktopRuntimeProvider gateway={runtime.value}>
          <DesktopRuntimeSurface placement="action-rail-safe" />
        </DesktopRuntimeProvider>
      </I18nextProvider>,
    );

    const trigger = await screen.findByRole('button', { name: /Local agents/u });
    expect(trigger.closest('aside')).toHaveAttribute('data-placement', 'action-rail-safe');
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
  });

  it('设备注册失败保留原因和显式重试，不再提示用户批准旧授权', async () => {
    const runtime = gateway({
      authorization: null,
      session: null,
      lifecycle: {
        ...authorizationRuntime.lifecycle,
        diagnosticCode: 'desktop.authorization.failed',
        lastFailureCode: 'bridge.identity_assertion_invalid',
        phase: 'halted',
      },
      deviceReauthorizationAvailable: false,
    });
    render(
      <I18nextProvider i18n={i18n}>
        <DesktopRuntimeProvider gateway={runtime.value}>
          <LocalConnectionNotice />
          <DesktopRuntimeSurface />
        </DesktopRuntimeProvider>
      </I18nextProvider>,
    );
    expect(
      await screen.findByText(
        'Device authorization could not finish. Automatic retries have stopped. Try connecting again.',
      ),
    ).toBeVisible();
    expect(screen.getByText('bridge.identity_assertion_invalid')).not.toBeVisible();
    fireEvent.click(screen.getByText('Connection and identity details'));
    expect(screen.getByText('bridge.identity_assertion_invalid')).toBeVisible();
    expect(runtime.retryBridge).not.toHaveBeenCalled();
    expect(runtime.openAuthorization).not.toHaveBeenCalled();
    fireEvent.click(await screen.findByRole('button', { name: /Local agents/u }));
    await waitFor(() => {
      expect(
        screen.getByRole('heading', { name: 'This computer is not connected yet' }),
      ).toBeVisible();
    });
    expect(screen.queryByRole('button', { name: 'Open secure sign-in' })).not.toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Re-authorize this computer' }),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Reconnect local agents' }));
    await waitFor(() => {
      expect(runtime.retryBridge).toHaveBeenCalledTimes(1);
    });
  });

  it('设备验证码过期时说明原因并引导重试获取新代码，而不是笼统提示授权失败', async () => {
    const runtime = gateway({
      authorization: null,
      session: null,
      lifecycle: {
        ...authorizationRuntime.lifecycle,
        diagnosticCode: 'desktop.authorization.failed',
        lastFailureCode: 'bridge.authorization_expired',
        phase: 'halted',
      },
      deviceReauthorizationAvailable: false,
    });
    const expired =
      'The one-time code expired before it was approved. Try connecting again to get a new code.';
    render(
      <I18nextProvider i18n={i18n}>
        <DesktopRuntimeProvider gateway={runtime.value}>
          <LocalConnectionNotice />
          <DesktopRuntimeSurface />
        </DesktopRuntimeProvider>
      </I18nextProvider>,
    );

    expect(await screen.findByText(expired)).toBeVisible();
    expect(
      screen.queryByText(
        'Device authorization could not finish. Automatic retries have stopped. Try connecting again.',
      ),
    ).not.toBeInTheDocument();
    fireEvent.click(await screen.findByRole('button', { name: /Local agents/u }));
    await waitFor(() => {
      expect(screen.getAllByText(expired)).toHaveLength(2);
    });
    expect(runtime.openAuthorization).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Reconnect local agents' }));
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
      deviceReauthorizationAvailable: false,
    };
    // 正式版本跟随稳定渠道；启动时的自动检查用的也是这个渠道，显式点“检查”再查一次。
    const runtime = gateway(ready, true, '0.1.0');
    runtime.checkUpdate.mockResolvedValueOnce(
      ok({
        available: false,
        channel: 'stable',
        currentVersion: '0.1.0',
        rollback: false,
        sequence: 7,
        targetVersion: '0.1.0',
      }),
    );
    render(
      <I18nextProvider i18n={i18n}>
        <DesktopRuntimeProvider gateway={runtime.value}>
          <DesktopRuntimeSurface />
        </DesktopRuntimeProvider>
      </I18nextProvider>,
    );

    await waitFor(() => {
      expect(runtime.checkUpdate).toHaveBeenCalledWith('stable');
    });
    fireEvent.click(await screen.findByRole('button', { name: /Local agents/u }));
    fireEvent.click(await screen.findByRole('button', { name: 'Check' }));
    await waitFor(() => {
      expect(runtime.checkUpdate).toHaveBeenCalledWith('stable');
      expect(runtime.checkUpdate).toHaveBeenCalledTimes(2);
      expect(screen.getByText('0.1.0 → 0.2.0')).toBeVisible();
    });
    fireEvent.click(screen.getByRole('button', { name: 'Install & restart' }));
    await waitFor(() => {
      expect(runtime.installUpdate).toHaveBeenCalledWith('stable', 8);
    });
  });

  it('为未识别宿主展示真实安装路径并复制通用 STDIO 配置', async () => {
    const ready: BridgeRuntime = {
      authorization: null,
      session: null,
      lifecycle: {
        ...authorizationRuntime.lifecycle,
        diagnosticCode: 'desktop.bridge.ready',
        phase: 'ready',
      },
      deviceReauthorizationAvailable: false,
    };
    const writeText = vi.fn(() => Promise.resolve(undefined));
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    });
    const runtime = gateway(ready);
    render(
      <I18nextProvider i18n={i18n}>
        <DesktopRuntimeProvider gateway={runtime.value}>
          <DesktopRuntimeSurface />
        </DesktopRuntimeProvider>
      </I18nextProvider>,
    );

    fireEvent.click(await screen.findByRole('button', { name: /Local agents/u }));
    fireEvent.click(screen.getByText('MCP compatibility'));
    fireEvent.click(screen.getByRole('button', { name: 'Other MCP hosts' }));
    expect(screen.getByText('C:\\Agent Room\\agent-room-mcp.exe')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Copy JSON' }));

    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith(expect.stringContaining('"agent_room"'));
      expect(screen.getByRole('button', { name: 'Copied' })).toBeVisible();
    });
  });
});
