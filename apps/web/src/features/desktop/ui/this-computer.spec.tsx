// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { I18nextProvider } from 'react-i18next';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type {
  BridgeRuntime,
  DesktopRuntimeEventHandlers,
  DesktopRuntimeGateway,
  DesktopRuntimeSnapshot,
  HostSessionDiagnostics,
  ReleaseUpdateCheck,
  ReleaseUpdateStatus,
} from '@/features/desktop/domain/desktop-runtime';
import { DesktopRuntimeProvider } from '@/features/desktop/ui/desktop-runtime-provider';
import { LocalAgentSessions } from '@/features/desktop/ui/local-agent-sessions';
import { ThisComputerBanner } from '@/features/desktop/ui/this-computer-banner';
import { ThisComputerSection } from '@/features/desktop/ui/this-computer-section';
import { ThisComputerStatus } from '@/features/desktop/ui/this-computer-status';
import { ThisComputerSettings } from '@/features/desktop/ui/this-computer-settings';
import { ApplicationUpdates } from '@/features/updates/ui/application-updates';
import { DesktopUpdateToast } from '@/features/updates/ui/desktop-update-toast';
import { AppNavigation } from '@/shared/ui/app-navigation';
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

const availableUpdate: ReleaseUpdateCheck = {
  available: true,
  channel: 'testing',
  currentVersion: '0.1.0',
  rollback: false,
  sequence: 8,
  targetVersion: '0.2.0',
};

function updateStatus(overrides: Partial<ReleaseUpdateStatus> = {}): ReleaseUpdateStatus {
  return {
    channel: 'testing',
    check: availableUpdate,
    checkedAtUnixMs: Date.now() - 60_000,
    failure: null,
    ...overrides,
  };
}

function snapshot(
  bridge: BridgeRuntime,
  options: {
    readonly currentVersion?: string;
    readonly updateStatus?: ReleaseUpdateStatus | null;
    readonly appTranslocated?: boolean;
  } = {},
): DesktopRuntimeSnapshot {
  return {
    autostartEnabled: false,
    bridge,
    ...(options.currentVersion === undefined ? {} : { currentVersion: options.currentVersion }),
    updateStatus: options.updateStatus ?? null,
    appTranslocated: options.appTranslocated ?? false,
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
    readonly updateStatus?: ReleaseUpdateStatus | null;
    readonly appTranslocated?: boolean;
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
  const installUpdate = vi.fn<DesktopRuntimeGateway['installUpdate']>(() =>
    Promise.resolve(ok(undefined)),
  );
  const openLogs = vi.fn(() => Promise.resolve(ok(undefined)));
  const events: { handlers: DesktopRuntimeEventHandlers | null } = { handlers: null };
  const value: DesktopRuntimeGateway = {
    beginHumanAuthentication: unavailable,
    beginMatrixAuthentication: unavailable,
    bootstrapDefaultAgent: unavailable,
    checkUpdate,
    clearHumanSession: () => Promise.resolve(ok(undefined)),
    restoreHumanSession: () => Promise.resolve(ok(true)),
    sendControlPlaneRequest: () =>
      Promise.resolve(ok({ status: 204, headers: [], body: new Uint8Array() })),
    configureAgentRuntime: (target) => Promise.resolve(ok(target)),
    installUpdate,
    isAvailable: () => true,
    openAuthorization,
    openLogs,
    readHostSessions: () => Promise.resolve(ok(options.sessions ?? [])),
    retryBridge,
    reauthorizeBridge,
    setAutostart: (enabled) => Promise.resolve(ok(enabled)),
    snapshot: () => Promise.resolve(ok(snapshot(bridge, options))),
    subscribe: (handlers) => {
      events.handlers = handlers;
      return Promise.resolve(ok(() => undefined));
    },
  };
  return {
    checkUpdate,
    events,
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

  it('已连接时说一句能用了，并给“这台电脑的设置”的入口', async () => {
    renderDesktop(gateway(ready).value, <ThisComputerSection />);

    expect(
      await screen.findByText('Connected. Agents you start on this computer can join rooms.'),
    ).toBeVisible();
    expect(screen.getByRole('link', { name: 'Settings for this computer' })).toHaveAttribute(
      'href',
      '/settings/this-computer',
    );
    // 开机启动、日志和 MCP 配置都搬进了“设置”。
    expect(screen.queryByRole('button', { name: 'Open log folder' })).not.toBeInTheDocument();
  });

  it('设置里能开关自动打开、打开日志文件夹、复制通用 MCP 配置', async () => {
    const writeText = vi.fn(() => Promise.resolve(undefined));
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const runtime = gateway(ready);
    renderDesktop(runtime.value, <ThisComputerSettings />);

    const autostart = await screen.findByRole('button', { name: 'Off' });
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
    expect(document.body).not.toHaveTextContent(/Codex|Claude Code|Cursor/u);
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
  afterEach(() => {
    window.localStorage.clear();
    vi.useRealTimers();
  });

  it('原生层查到新版本：提示栈里一条，点一下就更新并重启，“设置”上一个提醒点；网页自己不查', async () => {
    const runtime = gateway(ready, {
      currentVersion: '0.1.0-alpha.47',
      updateStatus: updateStatus(),
    });
    renderDesktop(
      runtime.value,
      <>
        <AppNavigation />
        <DesktopUpdateToast />
      </>,
    );

    expect(await screen.findByText('Agent Room 0.2.0 is ready to install')).toBeVisible();
    expect(screen.getByRole('link', { name: /Settings.*Update ready/u })).toBeVisible();
    expect(runtime.checkUpdate).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Update and restart' }));
    await waitFor(() => {
      expect(runtime.installUpdate).toHaveBeenCalledWith('testing', 8);
    });
  });

  it('“稍后”记住这个版本 24 小时，到点再提醒；更新的版本马上提醒', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const runtime = gateway(ready, { updateStatus: updateStatus() });
    renderDesktop(
      runtime.value,
      <>
        <AppNavigation />
        <DesktopUpdateToast />
      </>,
    );
    fireEvent.click(await screen.findByRole('button', { name: 'Later' }));
    expect(screen.queryByText('Agent Room 0.2.0 is ready to install')).not.toBeInTheDocument();
    // “设置”上的点还在。
    expect(screen.getByRole('link', { name: /Settings.*Update ready/u })).toBeVisible();
    act(() => {
      vi.advanceTimersByTime(24 * 60 * 60 * 1_000);
    });
    expect(await screen.findByText('Agent Room 0.2.0 is ready to install')).toBeVisible();

    fireEvent.click(screen.getByRole('button', { name: 'Later' }));
    act(() => {
      runtime.events.handlers?.onUpdateStatus?.(
        updateStatus({
          check: { ...availableUpdate, sequence: 9, targetVersion: '0.3.0' },
          checkedAtUnixMs: Date.now(),
        }),
      );
    });
    expect(await screen.findByText('Agent Room 0.3.0 is ready to install')).toBeVisible();
  });

  it('托盘菜单点了“更新到 X…”：网页照常先看草稿再装，被“稍后”收起的提示出来显示进度', async () => {
    const runtime = gateway(ready, { updateStatus: updateStatus() });
    runtime.installUpdate.mockImplementationOnce(() => new Promise(() => undefined));
    renderDesktop(runtime.value, <DesktopUpdateToast />);
    fireEvent.click(await screen.findByRole('button', { name: 'Later' }));
    act(() => {
      runtime.events.handlers?.onUpdateRequested?.();
    });
    await waitFor(() => {
      expect(runtime.installUpdate).toHaveBeenCalledWith('testing', 8);
    });
    act(() => {
      runtime.events.handlers?.onUpdateProgress?.({
        phase: 'downloading',
        downloadedBytes: 21,
        totalBytes: 42,
      });
    });
    expect(await screen.findByRole('button', { name: 'Downloading 50%' })).toBeDisabled();
  });

  it('装不上时提示里说一声，错误码收进详情，可以再点一次', async () => {
    const runtime = gateway(ready, { updateStatus: updateStatus() });
    runtime.installUpdate.mockResolvedValueOnce(
      err({ code: 'desktop.update.download_failed', retryable: true }),
    );
    renderDesktop(runtime.value, <DesktopUpdateToast />);
    fireEvent.click(await screen.findByRole('button', { name: 'Update and restart' }));
    expect(
      await screen.findByText('The update didn’t install. Check your connection and try again.'),
    ).toBeVisible();
    expect(screen.getByText('desktop.update.download_failed')).not.toBeVisible();
    expect(screen.getByRole('button', { name: 'Update and restart' })).toBeEnabled();
  });

  it('Mac 上应用在只读位置运行：不给更新按钮，说清楚先拖进“应用程序”文件夹', async () => {
    const runtime = gateway(ready, { appTranslocated: true, updateStatus: updateStatus() });
    renderDesktop(
      runtime.value,
      <>
        <DesktopUpdateToast />
        <ApplicationUpdates />
      </>,
    );
    expect(await screen.findByText('Agent Room 0.2.0 is ready to install')).toBeVisible();
    expect(
      screen.getAllByText(/Move it into the Applications folder, then open it from there/u),
    ).toHaveLength(2);
    expect(screen.queryByRole('button', { name: 'Update and restart' })).not.toBeInTheDocument();
  });

  it('设置里写上次检查：已是最新、没查成（错误码收进详情）、清单过期不当成故障', async () => {
    const current = gateway(ready, {
      updateStatus: updateStatus({ check: { ...availableUpdate, available: false } }),
    });
    renderDesktop(current.value, <ApplicationUpdates />);
    expect(await screen.findByText(/^Last checked .+: you’re up to date\.$/u)).toBeVisible();
    cleanup();

    const offline = gateway(ready, {
      updateStatus: updateStatus({
        check: null,
        failure: { code: 'desktop.update.manifest_network', retryable: true },
      }),
    });
    renderDesktop(offline.value, <ApplicationUpdates />);
    expect(await screen.findByText(/the check didn’t go through/u)).toBeVisible();
    expect(screen.getByText('desktop.update.manifest_network')).not.toBeVisible();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    cleanup();

    const expired = gateway(ready, {
      updateStatus: updateStatus({
        check: null,
        failure: { code: 'desktop.update.manifest_expired', retryable: false },
      }),
    });
    renderDesktop(expired.value, <ApplicationUpdates />);
    expect(await screen.findByText(/This clears up with the next release\./u)).toBeVisible();
    expect(screen.queryByText('desktop.update.manifest_expired')).not.toBeInTheDocument();
  });

  it('未授权也能检查并安装更新，没查成后按钮恢复可重试；停机时同样能装', async () => {
    const runtime = gateway(authorizing);
    runtime.checkUpdate.mockRejectedValueOnce(new Error('transport unavailable'));
    renderDesktop(runtime.value, <ApplicationUpdates />);
    const check = await screen.findByRole('button', { name: 'Check' });
    fireEvent.click(check);
    expect(await screen.findByText(/the check didn’t go through/u)).toBeVisible();
    expect(screen.getByText('desktop.update.check_failed')).not.toBeVisible();
    expect(check).toBeEnabled();
    fireEvent.click(check);
    fireEvent.click(await screen.findByRole('button', { name: 'Update and restart' }));
    await waitFor(() => {
      expect(runtime.installUpdate).toHaveBeenCalledWith('testing', 8);
    });
    expect(runtime.openAuthorization).not.toHaveBeenCalled();
    cleanup();

    // 升级往往正是修复停机的办法：原生层上次查到的新版本，打开设置就能装。
    const stopped = gateway(halted, { updateStatus: updateStatus() });
    renderDesktop(stopped.value, <ApplicationUpdates />);
    fireEvent.click(await screen.findByRole('button', { name: 'Update and restart' }));
    await waitFor(() => {
      expect(stopped.installUpdate).toHaveBeenCalledWith('testing', 8);
    });
    expect(stopped.checkUpdate).not.toHaveBeenCalled();
  });
});
