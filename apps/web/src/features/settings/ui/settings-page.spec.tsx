// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { DesktopRuntimeGateway } from '@/features/desktop/domain/desktop-runtime';
import { DesktopRuntimeProvider } from '@/features/desktop/ui/desktop-runtime-provider';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';
import { RouterTestProvider } from '@/test/router-test-provider';
import { SettingsLayout, SettingsSectionContent } from './settings-page';

const preferences = vi.hoisted(() => ({
  setLobbyView: vi.fn(),
  lobbyView: 'scene',
}));
const personal = vi.hoisted(() => ({
  change: vi.fn(),
  doNotDisturbUntil: 0,
}));

vi.mock('@/features/preferences/ui/account-preferences-provider', () => ({
  useOptionalAccountPreferences: () => ({
    deviceLanguageOverride: 'account',
    retry: () => undefined,
    setDeviceLanguageOverride: () => undefined,
    setLanguage: () => ({ ok: true, value: undefined }),
    setLobbyView: preferences.setLobbyView,
    snapshot: { values: { language: 'system', lobbyView: preferences.lobbyView } },
  }),
}));
vi.mock('@/features/personal-workspace/ui/personal-workspace-provider', () => ({
  usePersonalWorkspace: () => ({
    change: personal.change,
    snapshot: { index: { doNotDisturbUntil: personal.doNotDisturbUntil } },
  }),
}));

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(() => {
  preferences.setLobbyView.mockReset();
  preferences.lobbyView = 'scene';
  personal.change.mockReset();
  personal.change.mockReturnValue(ok(undefined));
  personal.doNotDisturbUntil = 0;
});

afterEach(cleanup);

function desktopGateway(updateAvailable: boolean): DesktopRuntimeGateway {
  const unavailable = () => Promise.resolve(err({ code: 'test.unavailable', retryable: false }));
  return {
    beginHumanAuthentication: unavailable,
    beginMatrixAuthentication: unavailable,
    bootstrapDefaultAgent: unavailable,
    checkUpdate: (channel) =>
      Promise.resolve(
        ok({
          available: updateAvailable,
          channel,
          currentVersion: '0.1.0-alpha.56',
          rollback: false,
          sequence: 57,
          targetVersion: '0.1.0-alpha.57',
        }),
      ),
    clearHumanSession: () => Promise.resolve(ok(undefined)),
    restoreHumanSession: () => Promise.resolve(ok(true)),
    sendControlPlaneRequest: () =>
      Promise.resolve(ok({ status: 204, headers: [], body: new Uint8Array() })),
    configureAgentRuntime: (target) => Promise.resolve(ok(target)),
    installUpdate: unavailable,
    isAvailable: () => true,
    openAuthorization: unavailable,
    retryBridge: unavailable,
    reauthorizeBridge: unavailable,
    setAutostart: (enabled) => Promise.resolve(ok(enabled)),
    snapshot: () =>
      Promise.resolve(
        ok({
          agentTarget: null,
          autostartEnabled: false,
          bridge: {
            authorization: null,
            deviceReauthorizationAvailable: false,
            session: null,
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
          },
          cliConfiguration: null,
          currentVersion: '0.1.0-alpha.56',
          deepLink: null,
          manualHostConfiguration: {
            args: [],
            command: 'C:\\Agent Room\\agent-room-mcp.exe',
            serverName: 'agent_room',
            transport: 'stdio',
          },
          platform: 'windows',
          updatesConfigured: true,
        }),
      ),
    subscribe: () => Promise.resolve(ok(() => undefined)),
  };
}

function renderSettings(node: ReactNode, gateway?: DesktopRuntimeGateway) {
  return render(
    <RouterTestProvider>
      <I18nextProvider i18n={i18n}>
        {gateway === undefined ? (
          node
        ) : (
          <DesktopRuntimeProvider gateway={gateway}>{node}</DesktopRuntimeProvider>
        )}
      </I18nextProvider>
    </RouterTestProvider>,
  );
}

describe('设置', () => {
  it('网页端分通用、账户、安全、关于四节；当前一节标成当前页', () => {
    renderSettings(
      <SettingsLayout section="general">
        <SettingsSectionContent section="general" />
      </SettingsLayout>,
    );
    expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeVisible();
    const nav = screen.getByRole('navigation', { name: 'Settings sections' });
    expect(
      within(nav)
        .getAllByRole('link')
        .map((link) => link.textContent),
    ).toEqual(['General', 'Account', 'Security', 'About']);
    expect(within(nav).getByRole('link', { name: 'General' })).toHaveAttribute(
      'aria-current',
      'page',
    );
    expect(screen.getByRole('heading', { level: 2, name: 'General' })).toBeVisible();
    // 顶栏也叫“设置”，不再有“安全”这个入口。
    expect(screen.getByRole('link', { name: 'Settings' })).toHaveAttribute('aria-current', 'page');
  });

  it('桌面端多一节“这台电脑”，查到新版本时这一节和顶栏的“设置”都有提醒点', async () => {
    renderSettings(
      <SettingsLayout section="this-computer">
        <SettingsSectionContent section="this-computer" />
      </SettingsLayout>,
      desktopGateway(true),
    );
    const nav = screen.getByRole('navigation', { name: 'Settings sections' });
    expect(
      await within(nav).findByRole('link', { name: /This computer.*Update ready/u }),
    ).toBeVisible();
    expect(screen.getByRole('link', { name: /^Settings.*Update ready/u })).toBeVisible();
    // 开机启动、应用更新（选渠道、检查）、日志、MCP 都在这一节。
    expect(await screen.findByRole('button', { name: 'Off' })).toBeVisible();
    expect(screen.getByRole('heading', { name: 'Application updates' })).toBeVisible();
    expect(screen.getByRole('combobox', { name: 'Update channel' })).toBeVisible();
    expect(screen.getByRole('button', { name: 'Open log folder' })).toBeVisible();
    expect(screen.getByText('MCP compatibility')).toBeVisible();
  });

  it('通用：语言、进房间时的视图、暂停提醒一小时', () => {
    vi.useFakeTimers({ now: Date.UTC(2026, 8, 30, 8, 0, 0), toFake: ['Date'] });
    renderSettings(<SettingsSectionContent section="general" />);
    expect(screen.getByRole('combobox', { name: 'Language' })).toBeVisible();

    fireEvent.click(screen.getByRole('radio', { name: 'List' }));
    expect(preferences.setLobbyView).toHaveBeenCalledWith('list');

    fireEvent.click(screen.getByRole('button', { name: 'Pause for an hour' }));
    expect(personal.change).toHaveBeenCalledWith({
      kind: 'dnd',
      id: 'global',
      value: Date.UTC(2026, 8, 30, 9, 0, 0),
    });
    vi.useRealTimers();
  });

  it('暂停着时说到几点，可以现在恢复；保存不了时如实说', async () => {
    personal.doNotDisturbUntil = Date.now() + 30 * 60_000;
    personal.change.mockReturnValue(err({ code: 'personal.unavailable', retryable: true }));
    renderSettings(<SettingsSectionContent section="general" />);
    expect(screen.getByText(/^Paused until /u)).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Resume now' }));
    expect(personal.change).toHaveBeenCalledWith({ kind: 'dnd', id: 'global', value: 0 });
    expect(await screen.findByRole('alert')).toHaveTextContent('Couldn’t save this setting.');
  });

  it('关于：版本、网页端检查更新、更新记录；桌面端指向“这台电脑”', async () => {
    renderSettings(<SettingsSectionContent section="about" />);
    expect(screen.getByRole('heading', { name: 'Agent Room' })).toBeVisible();
    expect(screen.getByRole('heading', { name: 'Application updates' })).toBeVisible();
    expect(screen.getByRole('button', { name: 'Check' })).toBeEnabled();
    expect(screen.getByRole('link', { name: /Release notes & downloads/u })).toBeVisible();
    cleanup();

    renderSettings(<SettingsSectionContent section="about" />, desktopGateway(false));
    expect(await screen.findByRole('link', { name: 'This computer' })).toHaveAttribute(
      'href',
      '/settings/this-computer',
    );
    await waitFor(() => {
      expect(screen.queryByRole('heading', { name: 'Application updates' })).toBeNull();
    });
  });
});
