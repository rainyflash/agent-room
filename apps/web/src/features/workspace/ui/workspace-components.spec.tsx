// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { AgentInstance, ProductDevice } from '@/features/security/domain/access-management';
import type { OwnedAgent } from '@/features/workspace/domain/agent-directory';
import { projectAgentFleet } from '@/features/workspace/domain/agent-fleet';
import {
  bridgeWorkspaceStatus,
  projectWorkspaceConnectionHealth,
} from '@/features/workspace/domain/connection-health';
import { AccountWorkspaceView } from '@/features/workspace/ui/account-workspace-view';
import { AgentCardList } from '@/features/workspace/ui/agent-card-list';
import {
  AgentDetailsDialog,
  type AgentDeletionControl,
} from '@/features/workspace/ui/agent-details-dialog';
import { AgentGroups } from '@/features/workspace/ui/agent-groups';
import { AgentRoomsView, BringCandidates } from '@/features/workspace/ui/agent-rooms-section';
import { ConnectionStatusStrip } from '@/features/workspace/ui/connection-status-strip';
import { DeviceRail } from '@/features/workspace/ui/device-rail';
import { WorkspaceDiagnostics } from '@/features/workspace/ui/workspace-diagnostics';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { RouterTestProvider } from '@/test/router-test-provider';

const AGENT_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e44';
const SCOUT_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e45';
const DEVICE_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e47';
const REMOTE_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e4a';
const NOW = 1_700_000_020_000;

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(async () => {
  window.localStorage.clear();
  await i18n.changeLanguage('en');
});

afterEach(cleanup);

describe('账号工作区组件', () => {
  it('按真实投影分别展示四层连接，不伪造云端在线', () => {
    renderWithI18n(<ConnectionStatusStrip health={fixtureConnectionHealth()} />);

    const strip = screen.getByRole('region', { name: 'Service connections' });
    expect(within(strip).getByText('Control plane')).toBeVisible();
    expect(within(strip).getByText('Matrix sync')).toBeVisible();
    expect(within(strip).getByText('This device Bridge')).toBeVisible();
    expect(within(strip).getByText('Agent runtimes')).toBeVisible();
    expect(within(strip).getByText('Degraded')).toBeVisible();
    expect(within(strip).getByText('Offline')).toBeVisible();
    expect(within(strip).getByText('Not installed')).toBeVisible();
    expect(within(strip).getByText('Online')).toBeVisible();
  });

  it('把 Bridge 生命周期映射为稳定的产品状态', () => {
    expect(bridgeWorkspaceStatus(false, undefined)).toBe('unavailable');
    expect(bridgeWorkspaceStatus(true, 'starting')).toBe('connecting');
    expect(bridgeWorkspaceStatus(true, 'ready')).toBe('online');
    expect(bridgeWorkspaceStatus(true, 'retry_scheduled')).toBe('degraded');
    expect(bridgeWorkspaceStatus(true, 'stopped')).toBe('offline');
  });

  it('在独立诊断面板显示观测时间和稳定故障码', async () => {
    const user = userEvent.setup();
    renderWithI18n(<WorkspaceDiagnostics health={fixtureConnectionHealth()} orphanCount={0} />);

    expect(screen.getByText('Connection diagnostics')).toBeVisible();
    expect(screen.getByText('2 service layers need attention.')).toBeVisible();
    expect(screen.getByText('control.devices_failed')).not.toBeVisible();
    await user.click(screen.getByText('Connection diagnostics'));
    expect(screen.getByText('control.devices_failed')).toBeVisible();
    expect(screen.getAllByText('Not observed on this client')).toHaveLength(3);
  });

  it('详情里说清它在哪台设备上、是不是这台电脑，不出现适配器这类内部说法', () => {
    const fleet = fixtureFleet();
    renderWithI18n(
      <>
        <DeviceRail devices={fleet.devices} />
        {fleet.agents[0] === undefined ? null : (
          <AgentDetailsDialog agent={fleet.agents[0]} now={NOW} onClose={() => undefined} />
        )}
      </>,
    );

    expect(screen.getByRole('heading', { name: 'Your devices' })).toBeVisible();
    expect(screen.getAllByText('Studio workstation')).toHaveLength(2);
    expect(screen.getByText('This device')).toBeVisible();
    const dialog = screen.getByRole('dialog', { name: 'Build Agent' });
    expect(within(dialog).getByText('This computer')).toBeVisible();
    expect(within(dialog).getAllByText('Online now').length).toBeGreaterThan(0);
    expect(dialog).not.toHaveTextContent(/adapter|capability|codex/iu);
  });

  it('删除 Agent 先确认，没有最近登录时改为重新登录', async () => {
    const user = userEvent.setup();
    const onDelete = vi.fn();
    const onReauthenticate = vi.fn();
    const agent = fixtureFleet().agents[0];
    if (agent === undefined) throw new Error('fixture agent missing');
    const control: AgentDeletionControl = {
      canDelete: () => true,
      failure: null,
      onDelete,
      onReauthenticate,
      pendingAgentId: null,
      recentlyAuthenticated: true,
    };
    const details = (deletion: AgentDeletionControl) => (
      <I18nextProvider i18n={i18n}>
        <AgentDetailsDialog agent={agent} deletion={deletion} now={NOW} onClose={() => undefined} />
      </I18nextProvider>
    );
    const view = render(details(control));

    await user.click(screen.getByRole('button', { name: 'Delete agent' }));
    const confirm = screen.getByRole('region', { name: 'Delete agent' });
    expect(within(confirm).getByText('Delete Build Agent?')).toBeVisible();
    await user.click(within(confirm).getByRole('button', { name: 'Delete' }));
    expect(onDelete).toHaveBeenCalledWith(agent);

    view.rerender(
      details({ ...control, failure: { agentId: AGENT_ID, code: 'agent.shared_ownership' } }),
    );
    expect(screen.getByRole('alert')).toHaveTextContent('has other owners');

    view.rerender(details({ ...control, recentlyAuthenticated: false }));
    expect(screen.queryByRole('button', { name: 'Delete' })).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Sign in again' }));
    expect(onReauthenticate).toHaveBeenCalledOnce();
  });

  it('默认 Agent 不显示删除入口', () => {
    const agent = fixtureFleet().agents[0];
    if (agent === undefined) throw new Error('fixture agent missing');
    renderWithI18n(
      <AgentDetailsDialog
        agent={agent}
        deletion={{
          canDelete: () => false,
          failure: null,
          onDelete: () => undefined,
          onReauthenticate: () => undefined,
          pendingAgentId: null,
          recentlyAuthenticated: true,
        }}
        now={NOW}
        onClose={() => undefined}
      />,
    );
    expect(screen.queryByRole('button', { name: 'Delete agent' })).not.toBeInTheDocument();
  });

  it('卡片说在不在线、在哪儿；点开交给 URL 状态所有者', async () => {
    const user = userEvent.setup();
    const open = vi.fn();
    const fleet = projectAgentFleet({
      agents: [agent(), agent(SCOUT_ID, 'Research Scout')],
      currentMatrixDeviceId: 'WEB-CURRENT',
      devices: [device()],
      instances: [
        instance(),
        {
          ...instance(),
          agentId: SCOUT_ID,
          agentInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e49',
          device: { ...instance().device, deviceId: REMOTE_ID, label: 'Travel laptop' },
          lastSeenAtUnixMs: NOW - 3 * 3_600_000,
          status: 'offline',
        },
      ],
    });
    renderWithI18n(<AgentCardList agents={fleet.agents} now={NOW} onOpen={open} />);

    const build = screen.getByRole('button', { name: /Build Agent/u });
    expect(build).toHaveTextContent('Online now');
    expect(build).toHaveTextContent('On this computer');
    const scout = screen.getByRole('button', { name: /Research Scout/u });
    expect(scout).toHaveTextContent('Last online 3 hours ago');
    expect(scout).toHaveTextContent('On Travel laptop');
    await user.click(build);
    expect(open).toHaveBeenCalledWith(AGENT_ID);
  });

  it('在线的一组摊开，不在线的收起来、只写有几个；正在连接的算在线', async () => {
    const user = userEvent.setup();
    const fleet = projectAgentFleet({
      agents: [agent(), agent(SCOUT_ID, 'Research Scout'), agent(REMOTE_ID, 'Old Helper')],
      currentMatrixDeviceId: 'WEB-CURRENT',
      devices: [device()],
      instances: [
        { ...instance(), status: 'connecting' },
        {
          ...instance(),
          agentId: SCOUT_ID,
          agentInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e49',
          status: 'offline',
        },
      ],
    });
    renderWithI18n(<AgentGroups agents={fleet.agents} now={NOW} onOpen={vi.fn()} />);

    const online = screen.getByRole('region', { name: 'Online · 1' });
    expect(within(online).getByRole('button', { name: /Build Agent/u })).toBeVisible();
    const offline = screen.getByText('Offline · 2').closest('details');
    expect(offline).not.toHaveAttribute('open');
    await user.click(screen.getByText('Offline · 2'));
    expect(offline).toHaveAttribute('open');
    expect(
      within(offline as HTMLElement).getByRole('button', { name: /Research Scout/u }),
    ).toBeVisible();
    expect(
      within(offline as HTMLElement).getByRole('button', { name: /Old Helper/u }),
    ).toBeVisible();
  });

  it('一个在线的都没有时照样有在线那一组，说现在没有', () => {
    const fleet = projectAgentFleet({
      agents: [agent()],
      currentMatrixDeviceId: 'WEB-CURRENT',
      devices: [device()],
      instances: [{ ...instance(), status: 'offline' }],
    });
    renderWithI18n(<AgentGroups agents={fleet.agents} now={NOW} onOpen={vi.fn()} />);

    const online = screen.getByRole('region', { name: 'Online · 0' });
    expect(online).toHaveTextContent('None of your agents are online right now.');
    expect(screen.getByText('Offline · 1')).toBeVisible();
  });

  it('卡片上写它在几个房间，不在任何房间也说', () => {
    const fleet = projectAgentFleet({
      agents: [agent(), agent(SCOUT_ID, 'Research Scout')],
      currentMatrixDeviceId: 'WEB-CURRENT',
      devices: [device()],
      instances: [instance()],
    });
    renderWithI18n(
      <AgentCardList
        agents={fleet.agents}
        now={NOW}
        onOpen={() => undefined}
        roomCounts={
          new Map([
            [AGENT_ID, 2],
            [SCOUT_ID, 0],
          ])
        }
      />,
    );

    expect(screen.getByRole('button', { name: /Build Agent/u })).toHaveTextContent('In 2 rooms');
    expect(screen.getByRole('button', { name: /Research Scout/u })).toHaveTextContent(
      'Not in any room',
    );
  });

  it('详情里每个房间一行带“离开”，“带进房间”打开房间选择；网络 Agent 只看不动', async () => {
    const user = userEvent.setup();
    const onLeave = vi.fn();
    const onPick = vi.fn();
    const rooms = [
      {
        archived: false,
        catalogId: '0198b601-77a2-7f41-b4f4-940f291951b8',
        kind: 'public_lobby' as const,
        name: 'Builders Exchange',
      },
      {
        archived: true,
        catalogId: '0198b601-77a7-7bb8-83eb-a8fe68c97e60',
        kind: 'private_room' as const,
        name: 'Weekly review',
      },
    ];
    const view = (managed: boolean) => (
      <I18nextProvider i18n={i18n}>
        <AgentRoomsView
          agentName="Build Agent"
          managed={managed}
          notice={{ kind: 'brought', room: 'Design crit', entered: 0 }}
          onLeave={onLeave}
          onPick={onPick}
          onRetry={() => undefined}
          pendingCatalogId={null}
          picker={null}
          rooms={rooms}
          state="ready"
        />
      </I18nextProvider>
    );
    const rendered = render(view(true));

    const section = screen.getByRole('region', { name: 'Rooms' });
    expect(within(section).getByText('Public lobby')).toBeVisible();
    expect(within(section).getByText('Archived')).toBeVisible();
    expect(within(section).getByRole('status')).toHaveTextContent('no working connection');
    await user.click(
      within(section).getByRole('button', { name: 'Take Build Agent out of Weekly review' }),
    );
    expect(onLeave).toHaveBeenCalledWith(rooms[1]);
    await user.click(within(section).getByRole('button', { name: 'Take into a room' }));
    expect(onPick).toHaveBeenCalledWith(true);

    rendered.rerender(view(false));
    expect(screen.queryByRole('button', { name: /Take .* out of/u })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Take into a room' })).not.toBeInTheDocument();
    expect(screen.getByText(/network agent goes in and out of rooms on its own/u)).toBeVisible();
  });

  it('房间选择写明哪些房间它进去只能看', async () => {
    const user = userEvent.setup();
    const onBring = vi.fn();
    const crit = {
      catalogId: '0198b601-77a7-7bb8-83eb-a8fe68c97e61',
      kind: 'private_room' as const,
      name: 'Design crit',
      viewOnly: false,
    };
    renderWithI18n(
      <BringCandidates
        candidates={[
          crit,
          {
            catalogId: '0198b601-77a7-7bb8-83eb-a8fe68c97e62',
            kind: 'private_room',
            name: 'Board notes',
            viewOnly: true,
          },
        ]}
        onBring={onBring}
        state="ready"
      />,
    );

    const picker = screen.getByRole('group', { name: 'Which room?' });
    expect(within(picker).getByRole('button', { name: /Board notes/u })).toHaveTextContent(
      'can only read there',
    );
    expect(within(picker).getByRole('button', { name: /Design crit/u })).not.toHaveTextContent(
      'can only read there',
    );
    await user.click(within(picker).getByRole('button', { name: /Design crit/u }));
    expect(onBring).toHaveBeenCalledWith(crit);
  });

  it('还没有 Agent 时给一个接入按钮；页头的主按钮也是接入 Agent', async () => {
    const user = userEvent.setup();
    const invite = vi.fn();
    renderWithI18n(
      <RouterTestProvider>
        <AccountWorkspaceView
          accountName="Ada"
          connectionHealth={fixtureConnectionHealth()}
          failureCode={null}
          fleet={projectAgentFleet({
            agents: [],
            currentMatrixDeviceId: null,
            devices: [],
            instances: [],
          })}
          loading={false}
          onInvite={invite}
          onRefresh={() => undefined}
          onSelectAgent={() => undefined}
          selectedAgentId={null}
        />
      </RouterTestProvider>,
    );

    expect(screen.getByRole('heading', { name: 'Bring your first agent' })).toBeVisible();
    const buttons = screen.getAllByRole('button', { name: 'Bring an agent' });
    expect(buttons).toHaveLength(2);
    for (const button of buttons) await user.click(button);
    expect(invite).toHaveBeenCalledTimes(2);
    // 设备和服务连接这类排查信息默认收起。
    expect(screen.getByRole('region', { name: 'Service connections' })).not.toBeVisible();
  });

  it('设备目录为空时给出明确状态', () => {
    renderWithI18n(<DeviceRail devices={[]} />);

    expect(screen.getByText('No device has signed in to this account yet.')).toBeVisible();
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

function fixtureConnectionHealth() {
  return projectWorkspaceConnectionHealth({
    agents: { failureCode: null, fleet: fixtureFleet(), loading: false },
    bridge: {
      available: false,
      changedAtUnixMs: null,
      failureCode: null,
      phase: undefined,
    },
    controlPlane: {
      failureCode: 'control.devices_failed',
      observedAtUnixMs: null,
      pending: false,
      results: [{ ok: true }, { ok: false }, { ok: true }],
    },
    matrix: {
      failureCode: 'matrix.offline',
      observedAtUnixMs: null,
      pending: false,
      result: { ok: false },
    },
  });
}

function agent(agentId = AGENT_ID, displayName = 'Build Agent'): OwnedAgent {
  return {
    agentId,
    avatarContentId: null,
    description: 'Builds and verifies releases.',
    displayName,
    matrixUserId: `@_agent_${agentId.slice(-4)}:matrix.test`,
    registeredAtUnixMs: 1_700_000_000_000,
    slug: displayName.toLowerCase().replaceAll(' ', '-'),
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
