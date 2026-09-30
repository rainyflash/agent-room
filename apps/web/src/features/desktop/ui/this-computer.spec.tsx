// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { I18nextProvider } from 'react-i18next';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type {
  BridgeRuntime,
  DesktopRuntimeGateway,
  DesktopRuntimeSnapshot,
  HostSessionDiagnostics,
} from '@/features/desktop/domain/desktop-runtime';
import { DesktopRuntimeProvider } from '@/features/desktop/ui/desktop-runtime-provider';
import { LocalAgentSessions } from '@/features/desktop/ui/local-agent-sessions';
import { ThisComputerBanner } from '@/features/desktop/ui/this-computer-banner';
import { ThisComputerSection } from '@/features/desktop/ui/this-computer-section';
import { ThisComputerStatus } from '@/features/desktop/ui/this-computer-status';
import { ApplicationAboutPage } from '@/features/updates/ui/application-about-page';
import { ApplicationVersionLink } from '@/features/updates/ui/application-version-link';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';
import { RouterTestProvider } from '@/test/router-test-provider';

const lifecycle: BridgeRuntime['lifecycle'] = {
  automaticRestartCount: 0,
  changedAtUnixMs: 1,
  diagnosticCode: 'desktop.bridge.ready',
  lastFailureCode: null,
  lastExitCode: null,
  nextRetryAtUnixMs: null,
  ownership: 'managed',
  phase: 'ready',
};
const ready: BridgeRuntime = {
  authorization: null,
  session: null,
  lifecycle,
  deviceReauthorizationAvailable: false,
};
const authorizing: BridgeRuntime = {
  ...ready,
  authorization: {
    expiresAtUnixMs: Date.now() + 600_000,
    promptId: 'authorization-7',
    userCode: 'ABCD-EFGH',
    verificationHost: 'identity.example',
  },
  lifecycle: { ...lifecycle, diagnosticCode: null, phase: 'authorization_required' },
};
const halted: BridgeRuntime = {
  ...ready,
  lifecycle: {
    ...lifecycle,
    automaticRestartCount: 4,
    diagnosticCode: 'desktop.bridge.restart_budget_exhausted',
    lastExitCode: 1,
    lastFailureCode: 'bridge.refresh_outcome_unknown',
    phase: 'halted',
  },
  deviceReauthorizationAvailable: true,
};

function snapshot(bridge: BridgeRuntime, currentVersion?: string): DesktopRuntimeSnapshot {
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
    updatesConfigured: true,
    agentTarget: null,
  };
}

function gateway(
  bridge: BridgeRuntime,
  options: {
    readonly currentVersion?: string;
    readonly sessions?: readonly HostSessionDiagnostics[];
  } = {},
) {
  const unavailable = () =>
    Promise.resolve(err({ code: 'desktop.test.unavailable', retryable: false }));
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
    beginHumanAuthentication: unavailable,
    beginMatrixAuthentication: unavailable,
    bootstrapDefaultAgent: unavailable,
    checkUpdate,
    clearHumanSession: () => Promise.resolve(ok(undefined)),
    restoreHumanSession: () => Promise.resolve(ok(true)),
    configureAgentRuntime: (target) => Promise.resolve(ok(target)),
    installUpdate,
    isAvailable: () => true,
    openAuthorization,
    openLogs,
    readHostSessions: () => Promise.resolve(ok(options.sessions ?? [])),
    readLobby: unavailable,
    retryBridge,
    reauthorizeBridge,
    setAutostart: (enabled) => Promise.resolve(ok(enabled)),
    snapshot: () => Promise.resolve(ok(snapshot(bridge, options.currentVersion))),
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

function renderDesktop(runtime: DesktopRuntimeGateway, node: ReactNode) {
  return render(
    <RouterTestProvider>
      <I18nextProvider i18n={i18n}>
        <DesktopRuntimeProvider gateway={runtime}>{node}</DesktopRuntimeProvider>
      </I18nextProvider>
    </RouterTestProvider>,
  );
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en-US']);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe('这台电脑', () => {
  it('要授权时只给身份站点和一次性代码，经闭合命令打开授权页；顶栏和页面上方都提醒', async () => {
    const runtime = gateway(authorizing);
    renderDesktop(
      runtime.value,
      <>
        <ThisComputerStatus />
        <ThisComputerBanner />
        <ThisComputerSection />
      </>,
    );

    const section = await screen.findByRole('region', { name: 'This computer' });
    expect(
      await within(section).findByText('Allow this computer to connect your agents'),
    ).toBeVisible();
    expect(within(section).getByText('identity.example')).toBeVisible();
    expect(within(section).getByText('ABCD-EFGH')).toBeVisible();
    expect(section).not.toHaveTextContent(/https:\/\//u);
    expect(screen.getByRole('link', { name: 'This computer: Needs authorization' })).toBeVisible();
    // 页面上方的提示同样带着代码，一个按钮就能打开授权页。
    const banners = screen.getAllByText(/Code ABCD-EFGH on identity\.example/u);
    expect(banners).toHaveLength(1);
    fireEvent.click(within(section).getByRole('button', { name: 'Open secure sign-in' }));
    await waitFor(() => {
      expect(runtime.openAuthorization).toHaveBeenCalledWith('authorization-7');
    });
  });

  it('自动重启停了：顶栏标“已停止”，提示条一个按钮重试，这一节里还能重新授权', async () => {
    const runtime = gateway(halted);
    renderDesktop(
      runtime.value,
      <>
        <ThisComputerStatus />
        <ThisComputerBanner />
        <ThisComputerSection />
      </>,
    );

    expect(await screen.findByRole('link', { name: 'This computer: Stopped' })).toBeVisible();
    const banner = screen.getByText('This computer’s connection stopped').closest('.ar-banner');
    if (!(banner instanceof HTMLElement)) throw new Error('banner missing');
    fireEvent.click(within(banner).getByRole('button', { name: 'Reconnect this computer' }));
    await waitFor(() => {
      expect(runtime.retryBridge).toHaveBeenCalledTimes(1);
    });

    const section = screen.getByRole('region', { name: 'This computer' });
    expect(within(section).getByText('Automatic restart was stopped')).toBeVisible();
    expect(within(section).getByText('bridge.refresh_outcome_unknown')).not.toBeVisible();
    fireEvent.click(within(section).getByRole('button', { name: 'Re-authorize this computer' }));
    await waitFor(() => {
      expect(runtime.reauthorizeBridge).toHaveBeenCalledTimes(1);
    });
    expect(runtime.retryBridge).toHaveBeenCalledTimes(1);
  });

  it('“我的 Agent”页自己显示这一节，所以那里不再出页面上方的提示', async () => {
    renderDesktop(
      gateway(halted).value,
      <>
        <ThisComputerStatus />
        <ThisComputerBanner hidden />
      </>,
    );
    expect(await screen.findByRole('link', { name: 'This computer: Stopped' })).toBeVisible();
    expect(screen.queryByText('This computer’s connection stopped')).not.toBeInTheDocument();
  });

  it('连不上服务器时说明原因和下次尝试时间，不当作崩溃，也允许立即重试', async () => {
    const nextRetryAtUnixMs = Date.UTC(2026, 8, 18, 17, 8, 57);
    const runtime = gateway({
      ...ready,
      lifecycle: {
        ...lifecycle,
        diagnosticCode: 'desktop.bridge.server_unreachable',
        lastFailureCode: 'bridge.identity_provider_unavailable',
        nextRetryAtUnixMs,
        phase: 'retry_scheduled',
      },
    });
    renderDesktop(
      runtime.value,
      <>
        <ThisComputerStatus />
        <ThisComputerBanner />
        <ThisComputerSection />
      </>,
    );

    expect(
      await screen.findByText(
        'Can’t reach Agent Room right now. Retrying automatically; you don’t need to restart the app.',
      ),
    ).toBeVisible();
    const nextAttempt = new Intl.DateTimeFormat(i18n.resolvedLanguage, {
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    }).format(new Date(nextRetryAtUnixMs));
    expect(screen.getByText(`Next attempt at ${nextAttempt}`)).toBeVisible();
    expect(screen.getByRole('link', { name: 'This computer: Connecting' })).toBeVisible();
    // 不是崩溃：页面上方不出“连接停了”的提示。
    expect(screen.queryByText('This computer’s connection stopped')).not.toBeInTheDocument();
    expect(screen.getByText('bridge.identity_provider_unavailable')).not.toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Retry connection' }));
    await waitFor(() => {
      expect(runtime.retryBridge).toHaveBeenCalledTimes(1);
    });
  });

  it('授权没完成时说明原因并重试，不再让人批准旧授权；验证码过期单独说', async () => {
    const failed = (lastFailureCode: string) =>
      gateway({
        ...ready,
        lifecycle: {
          ...lifecycle,
          diagnosticCode: 'desktop.authorization.failed',
          lastFailureCode,
          phase: 'halted',
        },
      });
    const runtime = failed('bridge.identity_assertion_invalid');
    renderDesktop(runtime.value, <ThisComputerSection />);
    expect(await screen.findByText('This computer is not connected yet')).toBeVisible();
    expect(
      screen.getByText(
        'Device authorization could not finish. Automatic retries have stopped. Try connecting again.',
      ),
    ).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Open secure sign-in' })).not.toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Re-authorize this computer' }),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Reconnect this computer' }));
    await waitFor(() => {
      expect(runtime.retryBridge).toHaveBeenCalledTimes(1);
    });
    cleanup();

    renderDesktop(failed('bridge.authorization_expired').value, <ThisComputerSection />);
    expect(
      await screen.findByText(
        'The one-time code expired before it was approved. Try connecting again to get a new code.',
      ),
    ).toBeVisible();
  });

  it('已连接时说一句能用了；设置里能开关自动打开、打开日志文件夹、复制通用 MCP 配置', async () => {
    const writeText = vi.fn(() => Promise.resolve(undefined));
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const runtime = gateway(ready);
    renderDesktop(runtime.value, <ThisComputerSection />);

    expect(
      await screen.findByText('Connected. Agents you start on this computer can join rooms.'),
    ).toBeVisible();
    const autostart = screen.getByRole('button', { name: 'Off' });
    fireEvent.click(autostart);
    expect(await screen.findByRole('button', { name: 'On' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    fireEvent.click(screen.getByRole('button', { name: 'Open log folder' }));
    await waitFor(() => {
      expect(runtime.openLogs).toHaveBeenCalledTimes(1);
    });
    fireEvent.click(screen.getByText('MCP compatibility'));
    expect(screen.getByText(/Add this JSON to the tool’s MCP configuration/u)).toBeVisible();
    expect(screen.getByRole('region', { name: 'This computer' })).not.toHaveTextContent(
      /Codex|Claude Code|Cursor/u,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Copy JSON' }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith(
        expect.stringContaining('C:\\\\Agent Room\\\\agent-room-mcp.exe'),
      );
    });
  });

  it('这台电脑上的 Agent：说它在不在看消息；读不到时如实说，不留过期的列表', async () => {
    const scout: HostSessionDiagnostics = {
      displayName: 'Scout',
      session: {
        sessionId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
        state: 'ready',
        agentId: null,
        errorCode: null,
      },
      lastInboxReadAgoMs: 1_000,
      lastMessageReceivedAgoMs: null,
      lastMessageSentAgoMs: 1_000,
    };
    const view = render(
      <I18nextProvider i18n={i18n}>
        <LocalAgentSessions readHostSessions={() => Promise.resolve(ok([scout]))} />
      </I18nextProvider>,
    );
    expect(await screen.findByText('Scout')).toBeVisible();
    expect(screen.getByText('In room')).toBeVisible();
    expect(screen.getByText('Reading messages')).toBeVisible();

    view.rerender(
      <I18nextProvider i18n={i18n}>
        <LocalAgentSessions
          readHostSessions={() =>
            Promise.resolve(err({ code: 'bridge.ipc.bridge_unavailable', retryable: true }))
          }
        />
      </I18nextProvider>,
    );
    expect(await screen.findByText('Can’t check this computer’s agents right now.')).toBeVisible();
    expect(screen.getByText('bridge.ipc.bridge_unavailable')).not.toBeVisible();
    expect(screen.queryByText('Scout')).not.toBeInTheDocument();
  });
});

describe('应用更新', () => {
  it('启动后自动按本版所属渠道查一次更新，顶栏的版本号换成“有新版本”', async () => {
    const runtime = gateway(ready, { currentVersion: '0.1.0-alpha.47' });
    renderDesktop(runtime.value, <ApplicationVersionLink />);

    await waitFor(() => {
      expect(runtime.checkUpdate).toHaveBeenCalledWith('testing');
    });
    expect(runtime.checkUpdate).toHaveBeenCalledTimes(1);
    const link = await screen.findByRole('link', {
      name: 'Update 0.2.0 ready to install · About & updates',
    });
    expect(link).toHaveTextContent('Update ready');
  });

  it('未授权也能在关于页检查并安装更新，出错后按钮恢复可重试；停机时同样能装', async () => {
    const runtime = gateway(authorizing);
    runtime.checkUpdate.mockRejectedValueOnce(new Error('transport unavailable'));
    renderDesktop(runtime.value, <ApplicationAboutPage />);
    const check = await screen.findByRole('button', { name: 'Check' });
    fireEvent.click(check);
    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent('The update did not complete');
    });
    expect(check).toBeEnabled();
    fireEvent.click(check);
    fireEvent.click(await screen.findByRole('button', { name: 'Install and restart' }));
    await waitFor(() => {
      expect(runtime.installUpdate).toHaveBeenCalledWith('testing', 8);
    });
    expect(runtime.openAuthorization).not.toHaveBeenCalled();
    cleanup();

    // 升级往往正是修复停机的办法。预发行版跟随测试渠道，启动时已经查过一次。
    const stopped = gateway(halted, { currentVersion: '0.1.0-alpha.47' });
    renderDesktop(stopped.value, <ApplicationAboutPage />);
    await waitFor(() => {
      expect(stopped.checkUpdate).toHaveBeenCalledWith('testing');
    });
    fireEvent.click(await screen.findByRole('button', { name: 'Install and restart' }));
    await waitFor(() => {
      expect(stopped.installUpdate).toHaveBeenCalledWith('testing', 8);
    });
  });
});
