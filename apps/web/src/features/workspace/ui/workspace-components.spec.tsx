// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { AgentInstance, ProductDevice } from '@/features/security/domain/access-management';
import type { OwnedAgent } from '@/features/workspace/domain/agent-directory';
import { projectAgentFleet } from '@/features/workspace/domain/agent-fleet';
import { AgentFleetList } from '@/features/workspace/ui/agent-fleet-list';
import { AgentInspector } from '@/features/workspace/ui/agent-inspector';
import {
  bridgeWorkspaceStatus,
  ConnectionStatusStrip,
} from '@/features/workspace/ui/connection-status-strip';
import { DeviceRail } from '@/features/workspace/ui/device-rail';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

const AGENT_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e44';
const DEVICE_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e47';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(async () => {
  window.localStorage.clear();
  await i18n.changeLanguage('en');
});

afterEach(cleanup);

describe('账号工作区组件', () => {
  it('把云端连接与未安装的本机 Bridge 分开展示', () => {
    renderWithI18n(<ConnectionStatusStrip bridgeStatus="unavailable" />);

    const strip = screen.getByRole('region', { name: 'Service connections' });
    expect(within(strip).getByText('Control plane')).toBeVisible();
    expect(within(strip).getByText('Matrix sync')).toBeVisible();
    expect(within(strip).getByText('This device Bridge')).toBeVisible();
    expect(within(strip).getAllByText('Online')).toHaveLength(2);
    expect(within(strip).getByText('Not installed')).toBeVisible();
  });

  it('把 Bridge 生命周期映射为稳定的产品状态', () => {
    expect(bridgeWorkspaceStatus(false, undefined)).toBe('unavailable');
    expect(bridgeWorkspaceStatus(true, 'starting')).toBe('connecting');
    expect(bridgeWorkspaceStatus(true, 'ready')).toBe('online');
    expect(bridgeWorkspaceStatus(true, 'retry_scheduled')).toBe('degraded');
    expect(bridgeWorkspaceStatus(true, 'stopped')).toBe('offline');
    expect(bridgeWorkspaceStatus(true, 'future_phase')).toBe('degraded');
  });

  it('展示当前设备和同一 Agent 的运行实例', () => {
    const fleet = fixtureFleet();
    renderWithI18n(
      <>
        <DeviceRail devices={fleet.devices} />
        <AgentInspector agent={fleet.agents[0] ?? null} />
      </>,
    );

    expect(screen.getByRole('heading', { name: 'Registered devices' })).toBeVisible();
    expect(screen.getAllByText('Studio workstation')).toHaveLength(2);
    expect(screen.getAllByText('This device')).toHaveLength(2);
    expect(screen.getByRole('heading', { name: 'Build Agent' })).toBeVisible();
    expect(screen.getByText('codex adapter · capability 1.0')).toBeVisible();
  });

  it('Agent 选择通过回调交给 URL 状态所有者', async () => {
    const user = userEvent.setup();
    const select = vi.fn();
    const fleet = fixtureFleet();
    renderWithI18n(
      <AgentFleetList
        agents={fleet.agents}
        onRefresh={() => undefined}
        onSelectAgent={select}
        selectedAgentId={null}
      />,
    );

    await user.click(screen.getByRole('button', { name: /Build Agent/u }));

    expect(select).toHaveBeenCalledWith(AGENT_ID);
  });

  it('设备目录为空时给出明确状态', () => {
    renderWithI18n(<DeviceRail devices={[]} />);

    expect(
      screen.getByText('No product device has been registered for this account.'),
    ).toBeVisible();
  });
});

function renderWithI18n(node: React.ReactNode) {
  return render(<I18nextProvider i18n={i18n}>{node}</I18nextProvider>);
}

function fixtureFleet() {
  return projectAgentFleet({
    agents: [agent()],
    currentMatrixDeviceId: 'WEB-CURRENT',
    devices: [device()],
    instances: [instance()],
  });
}

function agent(): OwnedAgent {
  return {
    agentId: AGENT_ID,
    avatarContentId: null,
    description: 'Builds and verifies releases.',
    displayName: 'Build Agent',
    matrixUserId: '@_agent_build:matrix.test',
    registeredAtUnixMs: 1_700_000_000_000,
    slug: 'build-agent',
    visibility: 'private',
  };
}

function device(): ProductDevice {
  return {
    createdAtUnixMs: 1_700_000_000_000,
    deviceId: DEVICE_ID,
    label: 'Studio workstation',
    lastSeenAtUnixMs: 1_700_000_010_000,
    matrixDeviceId: 'WEB-CURRENT',
    platform: 'windows',
    revokedAtUnixMs: null,
    trustState: 'verified',
  };
}

function instance(): AgentInstance {
  return {
    adapterType: 'codex',
    agentAvatarContentId: null,
    agentDisplayName: 'Build Agent',
    agentId: AGENT_ID,
    agentInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e48',
    capabilityVersion: '1.0',
    createdAtUnixMs: 1_700_000_000_000,
    device: {
      deviceId: DEVICE_ID,
      label: 'Studio workstation',
      platform: 'windows',
      trustState: 'verified',
    },
    lastSeenAtUnixMs: 1_700_000_010_000,
    matrixDeviceId: 'AGENT-CURRENT',
    matrixDeviceRevokedAtUnixMs: null,
    revokedAtUnixMs: null,
    status: 'online',
  };
}
