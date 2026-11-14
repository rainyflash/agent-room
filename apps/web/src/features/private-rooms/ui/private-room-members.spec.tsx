// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { NetworkAgentLabelStore } from '@/features/lobby/application/network-agent-label-store';
import { NetworkAgentLabelsProvider } from '@/features/lobby/ui/network-agent-labels';
import type { PrivateRoomCoordinator } from '@/features/private-rooms/application/private-room-coordinator';
import type {
  JoinCode,
  PrivateRoom,
  PrivateRoomAgentAccess,
  PrivateRoomAgentKnock,
  PrivateRoomGateway,
} from '@/features/private-rooms/domain/private-room';
import type {
  PrivateRoomListing,
  PrivateRoomListingGateway,
} from '@/features/private-rooms/domain/private-room-listing';
import type { PublicRoomJoinGateway } from '@/features/private-rooms/domain/public-room-join';
import type { RoomRoleChangeGateway } from '@/features/private-rooms/domain/room-role-changes';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

import { PrivateRoomAgentAccess as AgentCodeSection } from './private-room-agent-access';
import { PrivateRoomMembers } from './private-room-members';

const OWNER = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';
const MEMBER = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';
const BANNED = '0198b601-77a1-7bb8-83eb-a8fe68c97e4a';

const room: PrivateRoom = {
  catalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e46',
  description: 'Private review',
  matrixRoomId: '!private:matrix.test',
  members: [
    {
      permissions: { capabilities: ['view', 'speak', 'invite', 'manage', 'automate'] },
      principalId: OWNER,
      status: 'joined',
    },
    {
      permissions: { capabilities: ['view', 'speak'] },
      principalId: MEMBER,
      status: 'joined',
    },
  ],
  name: 'Architecture room',
  ownerPrincipalId: OWNER,
  retentionDays: 30,
  roomInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e47',
  status: 'active',
  version: 2,
};

const AGENT = '0198b601-77a1-7bb8-83eb-a8fe68c97e48';
const CREATED = Date.UTC(2026, 8, 23, 6, 0);
/** 房间原来就有的口令；生成的新口令是 `K7P3-Q9XW-2DMA`。 */
const STORED_CODE = 'H8J9-K2M3-N4P5';

/**
 * 和服务器一样记着口令状态和敲门：生成、停用、移出、放行之后再查看拿到的是新状态。
 * `revealable` 为 false 时原来的口令是能再看之前生成的，看不到；新生成的都看得到。
 */
function gateway(
  initial: PrivateRoomAgentAccess = { agents: [], joinCode: null },
  waiting: readonly PrivateRoomAgentKnock[] = [],
  revealable = true,
) {
  const unavailable = () => Promise.resolve(err({ code: 'test.unavailable', retryable: false }));
  const rename = vi.fn((_catalogId: string, name: string) =>
    Promise.resolve(ok({ ...room, name })),
  );
  let access = initial;
  let code: JoinCode | null =
    initial.joinCode === null
      ? null
      : { code: STORED_CODE, createdAtUnixMs: initial.joinCode.createdAtUnixMs };
  let hidden = !revealable;
  const agentAccess = vi.fn(() => Promise.resolve(ok(access)));
  const generateJoinCode = vi.fn(() => {
    const generated = { code: 'K7P3-Q9XW-2DMA', createdAtUnixMs: CREATED + 60_000 };
    access = { ...access, joinCode: { createdAtUnixMs: generated.createdAtUnixMs } };
    code = generated;
    hidden = false;
    return Promise.resolve(ok(generated));
  });
  const revealJoinCode = vi.fn<PrivateRoomGateway['revealJoinCode']>(() =>
    Promise.resolve(
      code === null
        ? err({ code: 'join_code.not_found', retryable: false })
        : hidden
          ? err({ code: 'join_code.not_revealable', retryable: false })
          : ok(code),
    ),
  );
  const disableJoinCode = vi.fn(() => {
    access = { ...access, joinCode: null };
    code = null;
    return Promise.resolve(ok(undefined));
  });
  const removeCodeAgent = vi.fn((_catalogId: string, agentId: string) => {
    access = {
      ...access,
      agents: access.agents.map((agent) =>
        agent.agentId === agentId ? { ...agent, status: 'removed' } : agent,
      ),
    };
    return Promise.resolve(ok(undefined));
  });
  let knocks = waiting;
  const agentKnocks = vi.fn(() => Promise.resolve(ok(knocks)));
  const admitKnock = vi.fn((_catalogId: string, agentId: string) => {
    const knock = knocks.find((candidate) => candidate.agentId === agentId);
    if (knock === undefined) {
      return Promise.resolve(err({ code: 'agent_knock.not_found', retryable: false }));
    }
    knocks = knocks.filter((candidate) => candidate.agentId !== agentId);
    const agent = {
      agentId,
      displayName: knock.displayName,
      joinedAtUnixMs: CREATED + 120_000,
      ownerDisplayName: null,
      status: 'joined' as const,
      statusChangedAtUnixMs: CREATED + 120_000,
    };
    access = { ...access, agents: [...access.agents, agent] };
    return Promise.resolve(ok(agent));
  });
  // 默认是人建的房间；要看 Agent 建的，改它的回答。
  const origin = vi.fn<PrivateRoomGateway['origin']>(() =>
    Promise.resolve(ok({ createdByAgent: null, ownerIsNetworkAgent: false })),
  );
  const people = vi.fn<PrivateRoomGateway['people']>(() =>
    Promise.resolve(
      ok([
        { displayName: 'Ada', principalId: OWNER },
        { displayName: 'Grace', principalId: MEMBER },
        { displayName: 'Mallory', principalId: BANNED },
      ]),
    ),
  );
  const unban = vi.fn<PrivateRoomGateway['unban']>(() => Promise.resolve(ok(room)));
  const archive = vi.fn<PrivateRoomGateway['archive']>(() =>
    Promise.resolve(ok({ ...room, status: 'archived' as const })),
  );
  const transferOwnership = vi.fn<PrivateRoomGateway['transferOwnership']>(() =>
    Promise.resolve(ok(room)),
  );
  const updatePermissions = vi.fn<PrivateRoomGateway['updatePermissions']>(() =>
    Promise.resolve(ok(room)),
  );
  const value: PrivateRoomGateway = {
    accept: unavailable,
    admitKnock,
    agentAccess,
    agentKnocks,
    archive,
    ban: unavailable,
    create: unavailable,
    decline: unavailable,
    declineKnock: unavailable,
    disableJoinCode,
    generateJoinCode,
    inspect: unavailable,
    invite: unavailable,
    leave: unavailable,
    list: () => Promise.resolve(ok([])),
    origin,
    people,
    remove: unavailable,
    removeCodeAgent,
    rename,
    revealJoinCode,
    transferOwnership,
    unban,
    updatePermissions,
  };
  return {
    admitKnock,
    agentAccess,
    agentKnocks,
    archive,
    disableJoinCode,
    generateJoinCode,
    origin,
    people,
    removeCodeAgent,
    rename,
    revealJoinCode,
    transferOwnership,
    unban,
    updatePermissions,
    value,
  };
}

/** 公开了、要申请才能进、不加密的房间，Lin 在等批准。 */
const publicListing: PrivateRoomListing = {
  allowNetworkAgents: true,
  endToEndEncrypted: false,
  firstListedAtUnixMs: CREATED,
  joinMode: 'request',
  listed: true,
  memberPermissions: { capabilities: ['view', 'speak', 'automate'] },
  platformHold: null,
  shareCode: 'aB3xYz',
  shareUrl: 'https://agentroom.test/r/aB3xYz',
};
const listings: PrivateRoomListingGateway = {
  changeShareCode: () => Promise.resolve(ok(publicListing)),
  listing: () => Promise.resolve(ok(publicListing)),
  updateListing: () => Promise.resolve(ok(publicListing)),
};
const joins: PublicRoomJoinGateway = {
  answerJoinRequest: () => Promise.resolve(ok(undefined)),
  joinRequests: () =>
    Promise.resolve(
      ok([
        {
          displayName: 'Lin',
          message: '',
          principalId: '0198b601-77a1-7bb8-83eb-a8fe68c97e50',
          requestedAtUnixMs: CREATED,
        },
      ]),
    ),
};

/**
 * 和房间设置里一样，“成员”和“Agent 进门”两节同时挂着；只有能管理房间的人有“Agent 进门”一节。
 * `networkAgents` 是服务器认定的网络 Agent。
 */
function renderGovernance(
  principalId: string,
  rooms: PrivateRoomGateway,
  networkAgents: readonly string[] = [],
  {
    agentCountOf,
    onExitRoom = () => undefined,
    onReauthenticate = () => undefined,
    recentlyAuthenticated = false,
    roleChanges,
    subject = room,
    publicRoom = false,
  }: {
    agentCountOf?: (principalId: string) => number | null;
    onExitRoom?: () => void;
    onReauthenticate?: () => void;
    recentlyAuthenticated?: boolean;
    roleChanges?: RoomRoleChangeGateway;
    subject?: PrivateRoom;
    publicRoom?: boolean;
  } = {},
) {
  const labels = new NetworkAgentLabelStore(
    {
      lookup: (ids) => Promise.resolve(ok(new Set(ids.filter((id) => networkAgents.includes(id))))),
    },
    {
      schedule: (task) => {
        task();
      },
    },
  );
  return render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={new QueryClient()}>
        <NetworkAgentLabelsProvider store={labels}>
          <PrivateRoomMembers
            agentCountOf={agentCountOf}
            coordinator={{} as PrivateRoomCoordinator}
            {...(publicRoom ? { joins, listings } : {})}
            onExitRoom={onExitRoom}
            onReauthenticate={onReauthenticate}
            principalId={principalId}
            recentlyAuthenticated={recentlyAuthenticated}
            roleChanges={roleChanges}
            room={subject}
            rooms={rooms}
          />
          {principalId === OWNER ? <AgentCodeSection room={subject} rooms={rooms} /> : null}
        </NetworkAgentLabelsProvider>
      </QueryClientProvider>
    </I18nextProvider>,
  );
}

/** 换掉一个成员的权限或者加一个成员，别的照旧。 */
function roomWith(...members: PrivateRoom['members']): PrivateRoom {
  const changed = new Set(members.map((member) => member.principalId));
  return {
    ...room,
    members: [...room.members.filter((member) => !changed.has(member.principalId)), ...members],
  };
}

/** 成员名单里写着这个名字的那一行。 */
function rowOf(name: string) {
  const item = screen.getByText(name).closest('li');
  if (item === null) throw new Error(`名单里没有 ${name} 那一行。`);
  return within(item);
}

/** 这一行的身份选择框里有哪几种。 */
function roleChoices(select: HTMLElement): (string | null)[] {
  return within(select)
    .getAllByRole('option')
    .map((option) => option.textContent);
}

function codeAgent(agentId: string, ownerDisplayName: string | null) {
  return {
    agentId,
    displayName: 'Scout',
    joinedAtUnixMs: CREATED,
    ownerDisplayName,
    status: 'joined' as const,
    statusChangedAtUnixMs: CREATED,
  };
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en-US']);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe('私人房间的成员与 Agent 口令', () => {
  it('房主可以改名：名字没变或为空时不能提交', async () => {
    const rooms = gateway();
    renderGovernance(OWNER, rooms.value);
    const input = screen.getByLabelText('Room name');
    const button = screen.getByRole('button', { name: 'Rename' });
    expect(input).toHaveValue('Architecture room');
    expect(button).toBeDisabled();
    fireEvent.change(input, { target: { value: '   ' } });
    expect(button).toBeDisabled();
    fireEvent.change(input, { target: { value: '  Design review ' } });
    expect(button).toBeEnabled();
    fireEvent.click(button);
    await waitFor(() => {
      expect(rooms.rename).toHaveBeenCalledWith(room.catalogId, 'Design review');
    });
  });

  it('Agent 建的房间写上是谁建的；网络 Agent 建的，加密一栏照实说服务器能读到', async () => {
    const rooms = gateway();
    const built = renderGovernance(MEMBER, rooms.value);
    await waitFor(() => {
      expect(rooms.origin).toHaveBeenCalledWith(room.catalogId);
    });
    expect(screen.queryByText('Made by')).toBeNull();
    expect(screen.getByText('End-to-end encrypted')).toBeVisible();
    built.unmount();

    rooms.origin.mockResolvedValue(
      ok({ createdByAgent: { agentId: AGENT, displayName: 'Scout' }, ownerIsNetworkAgent: false }),
    );
    const local = renderGovernance(MEMBER, rooms.value);
    expect(await screen.findByText('Scout (an agent)')).toBeVisible();
    expect(screen.getByText('End-to-end encrypted')).toBeVisible();
    local.unmount();

    rooms.origin.mockResolvedValue(
      ok({ createdByAgent: { agentId: AGENT, displayName: null }, ownerIsNetworkAgent: true }),
    );
    renderGovernance(MEMBER, rooms.value);
    expect(await screen.findByText('An agent')).toBeVisible();
    // 房主是建房的网络 Agent：成员里写它，不写合成主体的 ID。
    expect(screen.getByText('A network agent')).toBeVisible();
    expect(screen.queryByText(OWNER)).toBeNull();
    expect(
      screen.getByText(
        'End-to-end encrypted. A network agent made this room, so the server can read what is said here.',
      ),
    ).toBeVisible();
  });

  it('问不到是谁建的：不写谁建的，加密一栏照旧', async () => {
    const rooms = gateway();
    rooms.origin.mockResolvedValue(err({ code: 'private_room.unreachable', retryable: true }));
    renderGovernance(MEMBER, rooms.value);
    await waitFor(() => {
      expect(rooms.origin).toHaveBeenCalled();
    });
    expect(screen.queryByText('Made by')).toBeNull();
    expect(screen.getByText('End-to-end encrypted')).toBeVisible();
  });

  it('成员写昵称，自己那一行标着“你”；账户 ID 收在每一行的“更多”里', async () => {
    const rooms = gateway();
    renderGovernance(MEMBER, rooms.value);
    expect(await screen.findByText('Ada')).toBeVisible();
    expect(rowOf('Grace').getByText('you')).toBeVisible();
    expect(rooms.people).toHaveBeenCalledWith(room.catalogId);
    expect(screen.queryByText(OWNER)).toBeNull();
    const more = screen.getByRole('button', { name: 'More about Ada' });
    expect(more).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(more);
    expect(more).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText(OWNER)).toBeVisible();
  });

  it('问不到名字时说看不到，不露出账户 ID；自己那一行写“你”', async () => {
    const rooms = gateway();
    rooms.people.mockResolvedValue(
      err({ code: 'room_people.dependency_unavailable', retryable: true }),
    );
    renderGovernance(MEMBER, rooms.value);
    expect(await screen.findByText('Name not available')).toBeVisible();
    expect(screen.getByText('You')).toBeVisible();
    expect(screen.queryByText(OWNER)).toBeNull();
  });

  it('最上面写着四种身份各是什么', () => {
    renderGovernance(MEMBER, gateway().value);
    for (const [role, detail] of [
      ['Owner', 'One per room. Decides everything.'],
      ['Admin', 'Chosen by the owner to help run the whole room.'],
      ['Member', 'Reads, speaks, and can bring their own agents.'],
      ['View only', 'Reads, but can’t speak.'],
    ] as const) {
      const term = screen.getAllByText(role).find((element) => element.tagName === 'DT');
      expect(term?.nextElementSibling).toHaveTextContent(detail);
    }
  });

  it('退出、移出、封禁的人留在名单里，身份那一格写状态，下面有一句说明', async () => {
    const plain = renderGovernance(OWNER, gateway().value);
    expect(await screen.findByText('Grace')).toBeVisible();
    expect(screen.queryByText(/^People who left, were removed or were banned/u)).toBeNull();
    plain.unmount();

    renderGovernance(OWNER, gateway().value, [], {
      subject: roomWith({
        permissions: { capabilities: [] },
        principalId: BANNED,
        status: 'banned',
      }),
    });
    expect(await screen.findByText('Mallory')).toBeVisible();
    const banned = rowOf('Mallory');
    expect(banned.getByText('Banned')).toBeVisible();
    expect(banned.queryByRole('combobox')).toBeNull();
    expect(screen.getByText(/^People who left, were removed or were banned/u)).toBeVisible();
  });

  it('带了几个 Agent 照房间页问到的写；不知道就不写', async () => {
    const unknown = renderGovernance(OWNER, gateway().value);
    expect(await screen.findByText('Grace')).toBeVisible();
    expect(screen.queryByText(/agents?$/u)).toBeNull();
    unknown.unmount();

    renderGovernance(OWNER, gateway().value, [], {
      agentCountOf: (principalId) => (principalId === MEMBER ? 2 : principalId === OWNER ? 1 : 0),
      subject: roomWith({
        permissions: { capabilities: ['view'] },
        principalId: BANNED,
        status: 'joined',
      }),
    });
    expect(await screen.findByText('Grace')).toBeVisible();
    expect(rowOf('Grace').getByText('2 agents')).toBeVisible();
    expect(rowOf('Ada').getByText('1 agent')).toBeVisible();
    expect(rowOf('Mallory').getByText('No agents')).toBeVisible();
  });

  it('邀请表单看“邀请”：只有“管理”的看不到，只有“邀请”的看得到、勾不上自己没有的', async () => {
    const rooms = gateway();
    const manager = renderGovernance(MEMBER, rooms.value, [], {
      subject: roomWith({
        permissions: { capabilities: ['view', 'manage'] },
        principalId: MEMBER,
        status: 'joined',
      }),
    });
    expect(screen.queryByRole('heading', { name: 'Invite someone' })).toBeNull();
    manager.unmount();

    renderGovernance(MEMBER, rooms.value, [], {
      subject: roomWith({
        permissions: { capabilities: ['view', 'speak', 'invite'] },
        principalId: MEMBER,
        status: 'joined',
      }),
    });
    expect(await screen.findByRole('heading', { name: 'Invite someone' })).toBeVisible();
    const form = within(screen.getByRole('group', { name: 'Invitation permissions' }));
    expect(form.getByRole('checkbox', { name: /^Speak/u })).toBeEnabled();
    expect(form.getByRole('checkbox', { name: /^Invite/u })).toBeEnabled();
    expect(form.getByRole('checkbox', { name: /^Manage/u })).toBeDisabled();
    expect(form.getByRole('checkbox', { name: /^Automate/u })).toBeDisabled();
  });

  it('账户 ID 要和服务器一样是 UUID v7', () => {
    renderGovernance(OWNER, gateway().value);
    const input = screen.getByLabelText('Account ID');
    const send = screen.getByRole('button', { name: 'Send invitation' });
    fireEvent.change(input, { target: { value: '6f9619ff-8b86-4011-b42d-00cf4fc964ff' } });
    expect(screen.getByText('That is not an account ID.')).toBeVisible();
    expect(send).toBeDisabled();
    fireEvent.change(input, { target: { value: '0198b601-77a1-7bb8-83eb-a8fe68c97e4b' } });
    expect(send).toBeEnabled();
  });

  it('邀请时“管理”谁都勾不上，房主也一样：要设管理员到名单里选', async () => {
    renderGovernance(OWNER, gateway().value, [], { recentlyAuthenticated: true });
    const form = within(await screen.findByRole('group', { name: 'Invitation permissions' }));
    expect(form.getByRole('checkbox', { name: /^Manage/u })).toBeDisabled();
    expect(form.getByRole('checkbox', { name: /^Manage/u })).not.toBeChecked();
    expect(form.getByRole('checkbox', { name: /^Invite/u })).toBeEnabled();
    expect(form.getByRole('checkbox', { name: /^Automate/u })).toBeEnabled();
  });

  it('能管理的人解除封禁；不能管理的看不到这个按钮', async () => {
    const banned = roomWith({
      permissions: { capabilities: [] },
      principalId: BANNED,
      status: 'banned',
    });
    const rooms = gateway();
    const member = renderGovernance(MEMBER, rooms.value, [], { subject: banned });
    fireEvent.click(await screen.findByRole('button', { name: 'More about Mallory' }));
    expect(screen.queryByRole('button', { name: 'Unban' })).toBeNull();
    member.unmount();

    renderGovernance(OWNER, rooms.value, [], { subject: banned });
    fireEvent.click(await screen.findByRole('button', { name: 'More about Mallory' }));
    fireEvent.click(screen.getByRole('button', { name: 'Unban' }));
    await waitFor(() => {
      expect(rooms.unban).toHaveBeenCalledWith(room.catalogId, BANNED);
    });
  });

  it('转移房主先确认，默认留着全部权限，能少留几项，也能取消', async () => {
    const rooms = gateway();
    renderGovernance(OWNER, rooms.value, [], { recentlyAuthenticated: true });
    fireEvent.click(await screen.findByRole('button', { name: 'More about Grace' }));

    fireEvent.click(screen.getByRole('button', { name: 'Transfer ownership' }));
    const confirm = within(screen.getByRole('group', { name: 'Make Grace the owner?' }));
    fireEvent.click(confirm.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('group', { name: 'Make Grace the owner?' })).toBeNull();
    expect(rooms.transferOwnership).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: 'Transfer ownership' }));
    const again = within(screen.getByRole('group', { name: 'Make Grace the owner?' }));
    const kept = within(again.getByRole('group', { name: 'You keep' }));
    expect(kept.getByRole('checkbox', { name: /^Manage/u })).toBeChecked();
    fireEvent.click(kept.getByRole('checkbox', { name: /^Manage/u }));
    fireEvent.click(again.getByRole('button', { name: 'Transfer' }));
    await waitFor(() => {
      expect(rooms.transferOwnership).toHaveBeenCalledWith(room.catalogId, {
        formerOwnerPermissions: { capabilities: ['view', 'speak', 'invite', 'automate'] },
        targetPrincipalId: MEMBER,
      });
    });
  });

  it('对方已有公开房间时，确认框里说清楚转不过去，取消以后说明一起收掉', async () => {
    const rooms = gateway();
    rooms.transferOwnership.mockResolvedValueOnce(
      err({ code: 'private_room.public_limit_reached', retryable: false }),
    );
    renderGovernance(OWNER, rooms.value, [], { recentlyAuthenticated: true });
    fireEvent.click(await screen.findByRole('button', { name: 'More about Grace' }));

    fireEvent.click(screen.getByRole('button', { name: 'Transfer ownership' }));
    const confirm = within(screen.getByRole('group', { name: 'Make Grace the owner?' }));
    fireEvent.click(confirm.getByRole('button', { name: 'Transfer' }));

    expect(await confirm.findByText(/^Grace already has a public room/u)).toBeVisible();
    expect(screen.queryByText('private_room.public_limit_reached')).toBeNull();

    fireEvent.click(confirm.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByText(/already has a public room/u)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Transfer ownership' }));
    expect(screen.queryByText(/already has a public room/u)).toBeNull();
  });

  it('转移房主要最近登录过：不是的话先请人重新登录，不出确认', async () => {
    const rooms = gateway();
    const onReauthenticate = vi.fn();
    renderGovernance(OWNER, rooms.value, [], { onReauthenticate });
    fireEvent.click(await screen.findByRole('button', { name: 'More about Grace' }));

    fireEvent.click(screen.getByRole('button', { name: 'Transfer ownership' }));
    expect(await screen.findByText(/^This needs a fresh sign-in/u)).toBeVisible();
    expect(screen.queryByRole('group', { name: 'Make Grace the owner?' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Sign in again' }));
    expect(onReauthenticate).toHaveBeenCalledOnce();
    expect(rooms.transferOwnership).not.toHaveBeenCalled();
  });

  it('归档不要最近登录过：先在原处确认，取消不归档，确认以后归档并退出房间', async () => {
    const rooms = gateway();
    const onExitRoom = vi.fn();
    renderGovernance(OWNER, rooms.value, [], { onExitRoom });
    expect(await screen.findByText('Grace')).toBeVisible();

    const archive = screen.getByRole('button', { name: 'Archive room' });
    expect(archive).toBeEnabled();
    fireEvent.click(archive);
    const confirm = within(screen.getByRole('group', { name: 'Archive “Architecture room”?' }));
    expect(confirm.getByRole('button', { name: 'Cancel' })).toHaveFocus();
    fireEvent.click(confirm.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('group', { name: 'Archive “Architecture room”?' })).toBeNull();
    expect(archive).toHaveFocus();
    expect(rooms.archive).not.toHaveBeenCalled();

    fireEvent.click(archive);
    fireEvent.click(
      within(screen.getByRole('group', { name: 'Archive “Architecture room”?' })).getByRole(
        'button',
        { name: 'Archive' },
      ),
    );
    await waitFor(() => {
      expect(onExitRoom).toHaveBeenCalledOnce();
    });
    expect(rooms.archive).toHaveBeenCalledWith(room.catalogId);
  });

  it('成员没有归档，只有退出', async () => {
    renderGovernance(MEMBER, gateway().value);
    expect(await screen.findByRole('button', { name: 'Leave room' })).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Archive room' })).toBeNull();
  });

  it('房主选一下就设、撤管理员：设的时候一起给上邀请，撤的时候只去掉“管理”', async () => {
    const rooms = gateway();
    const plain = renderGovernance(OWNER, rooms.value, [], { recentlyAuthenticated: true });
    const role = await screen.findByRole('combobox', { name: 'Role of Grace' });
    expect(role).toHaveValue('member');
    expect(roleChoices(role)).toEqual(['Admin', 'Member', 'View only']);
    fireEvent.change(role, { target: { value: 'admin' } });
    await waitFor(() => {
      expect(rooms.updatePermissions).toHaveBeenCalledWith(room.catalogId, MEMBER, {
        capabilities: ['view', 'speak', 'invite', 'manage'],
      });
    });
    plain.unmount();

    renderGovernance(OWNER, rooms.value, [], {
      recentlyAuthenticated: true,
      subject: roomWith({
        permissions: { capabilities: ['view', 'speak', 'invite', 'manage'] },
        principalId: MEMBER,
        status: 'joined',
      }),
    });
    const admin = await screen.findByRole('combobox', { name: 'Role of Grace' });
    expect(admin).toHaveValue('admin');
    fireEvent.change(admin, { target: { value: 'member' } });
    await waitFor(() => {
      expect(rooms.updatePermissions).toHaveBeenLastCalledWith(room.catalogId, MEMBER, {
        capabilities: ['view', 'speak', 'invite'],
      });
    });
  });

  it('设、撤管理员要最近登录过：不是的话先请人重新登录，不发请求；改成只能看不用', async () => {
    const rooms = gateway();
    const onReauthenticate = vi.fn();
    renderGovernance(OWNER, rooms.value, [], { onReauthenticate });
    const role = await screen.findByRole('combobox', { name: 'Role of Grace' });
    fireEvent.change(role, { target: { value: 'admin' } });

    expect(await screen.findByText(/^This needs a fresh sign-in/u)).toBeVisible();
    expect(role).toHaveValue('member');
    expect(rooms.updatePermissions).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Sign in again' }));
    expect(onReauthenticate).toHaveBeenCalledOnce();

    fireEvent.click(rowOf('Grace').getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByText(/^This needs a fresh sign-in/u)).toBeNull();
    fireEvent.change(role, { target: { value: 'viewer' } });
    await waitFor(() => {
      expect(rooms.updatePermissions).toHaveBeenCalledWith(room.catalogId, MEMBER, {
        capabilities: ['view'],
      });
    });
  });

  it('页面以为算最近登录过、服务器说不算时，照样请人重新登录', async () => {
    const rooms = gateway();
    rooms.updatePermissions.mockResolvedValueOnce(
      err({ code: 'authentication.reauthentication_required', retryable: false }),
    );
    const onReauthenticate = vi.fn();
    renderGovernance(OWNER, rooms.value, [], { onReauthenticate, recentlyAuthenticated: true });
    fireEvent.change(await screen.findByRole('combobox', { name: 'Role of Grace' }), {
      target: { value: 'admin' },
    });

    expect(await screen.findByText(/^This needs a fresh sign-in/u)).toBeVisible();
    expect(screen.queryByText('authentication.reauthentication_required')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Sign in again' }));
    expect(onReauthenticate).toHaveBeenCalledOnce();
  });

  it('选了以后这一行马上照新的显示，不等服务器；服务器说不行就回到原来的', async () => {
    const rooms = gateway();
    type Answer = Awaited<ReturnType<PrivateRoomGateway['updatePermissions']>>;
    let answer: (result: Answer) => void = () => undefined;
    rooms.updatePermissions.mockReturnValueOnce(
      new Promise<Answer>((resolve) => {
        answer = resolve;
      }),
    );
    renderGovernance(OWNER, rooms.value, [], { recentlyAuthenticated: true });
    const role = await screen.findByRole('combobox', { name: 'Role of Grace' });
    fireEvent.change(role, { target: { value: 'viewer' } });

    expect(role).toHaveValue('viewer');
    answer(err({ code: 'private_room.owner_only', retryable: false }));
    await waitFor(() => {
      expect(role).toHaveValue('member');
    });
    // 名单上面那句说明也是这么开头的：多出来的一处是失败的原因。
    await waitFor(() => {
      expect(screen.getAllByText(/^Only the owner can make or remove admins/u)).toHaveLength(2);
    });
  });

  it('身份变动：房主和管理员在最下面看得到，归档了也看得到；成员看不到', async () => {
    const roleChanges = vi.fn<RoomRoleChangeGateway['roleChanges']>(() =>
      Promise.resolve(
        ok([
          {
            actorPrincipalId: OWNER,
            changedAtUnixMs: CREATED,
            id: 1,
            next: {
              owner: false,
              permissions: { capabilities: ['view', 'speak', 'invite', 'manage'] },
              status: 'joined',
            },
            previous: {
              owner: false,
              permissions: { capabilities: ['view', 'speak'] },
              status: 'joined',
            },
            targetPrincipalId: MEMBER,
          },
        ]),
      ),
    );
    const adminRoom = roomWith({
      permissions: { capabilities: ['view', 'speak', 'invite', 'manage'] },
      principalId: MEMBER,
      status: 'joined',
    });
    const readers: [string, PrivateRoom][] = [
      [OWNER, room],
      [MEMBER, adminRoom],
      [OWNER, { ...room, status: 'archived' }],
    ];
    for (const [reader, subject] of readers) {
      const view = renderGovernance(reader, gateway().value, [], {
        roleChanges: { roleChanges },
        subject,
      });
      const details = screen.getByText('Role changes').closest('details');
      if (details === null) throw new Error('“身份变动”没有收在详情里。');
      // 收着的时候不去读。
      expect(roleChanges).not.toHaveBeenCalled();
      details.open = true;
      fireEvent(details, new Event('toggle'));
      // 名字和成员名单用的是同一套。
      expect(await screen.findByText('Grace went from Member to Admin')).toBeInTheDocument();
      expect(within(details).getByText(/^By Ada · /u)).toBeInTheDocument();
      view.unmount();
      roleChanges.mockClear();
    }

    renderGovernance(MEMBER, gateway().value, [], { roleChanges: { roleChanges } });
    expect(await screen.findByText('Ada')).toBeVisible();
    expect(screen.queryByText('Role changes')).toBeNull();
    cleanup();
    // 夹具和测试里可以没有这项服务：没有就不出现。
    renderGovernance(OWNER, gateway().value);
    expect(await screen.findByText('Ada')).toBeVisible();
    expect(screen.queryByText('Role changes')).toBeNull();
    expect(roleChanges).not.toHaveBeenCalled();
  });

  it('不能管理的成员看得到每个人的身份，换不了', async () => {
    renderGovernance(MEMBER, gateway().value);
    expect(await screen.findByText('Ada')).toBeVisible();
    expect(rowOf('Ada').getByText('Owner')).toBeVisible();
    expect(rowOf('Grace').getByText('Member')).toBeVisible();
    expect(screen.queryByRole('combobox')).toBeNull();
    // 自己的两个开关看得到、动不了。
    fireEvent.click(screen.getByRole('button', { name: 'More about Grace' }));
    expect(screen.getByRole('switch', { name: 'Can invite people' })).toBeDisabled();
    expect(screen.queryByRole('button', { name: 'Remove' })).toBeNull();
  });

  it('管理员设不了管理员，也动不了另一位管理员；成员他照旧管得了', async () => {
    const PLAIN = '0198b601-77a1-7bb8-83eb-a8fe68c97e4b';
    renderGovernance(MEMBER, gateway().value, [], {
      subject: roomWith(
        {
          permissions: { capabilities: ['view', 'speak', 'invite', 'manage'] },
          principalId: MEMBER,
          status: 'joined',
        },
        {
          permissions: { capabilities: ['view', 'speak', 'manage'] },
          principalId: BANNED,
          status: 'joined',
        },
        { permissions: { capabilities: ['view', 'speak'] }, principalId: PLAIN, status: 'joined' },
      ),
    });
    expect(await screen.findByText('Mallory')).toBeVisible();

    // 另一位管理员那一行：只写“管理员”，没有选择框；“更多”里没有移除、封禁，开关动不了。
    const admin = rowOf('Mallory');
    expect(admin.getByText('Admin')).toBeVisible();
    expect(admin.queryByRole('combobox')).toBeNull();
    fireEvent.click(admin.getByRole('button', { name: 'More about Mallory' }));
    expect(admin.queryByRole('button', { name: 'Remove' })).toBeNull();
    expect(admin.queryByRole('button', { name: 'Ban' })).toBeNull();
    expect(admin.getByRole('switch', { name: 'Can invite people' })).toBeDisabled();

    // 成员那一行：在成员和只能看之间换，设不成管理员；移除、封禁、开关照旧。
    const plain = rowOf('Name not available');
    expect(roleChoices(plain.getByRole('combobox'))).toEqual(['Member', 'View only']);
    fireEvent.click(plain.getByRole('button', { name: /^More about/u }));
    expect(plain.getByRole('button', { name: 'Remove' })).toBeEnabled();
    expect(plain.getByRole('button', { name: 'Ban' })).toBeEnabled();
    expect(plain.getByRole('switch', { name: 'Can invite people' })).toBeEnabled();
    // 他自己没有自动发言，给不出。
    expect(plain.getByRole('switch', { name: 'Agents they bring can speak' })).toBeDisabled();
  });

  it('管理员自己不当了：先确认，确认了只去掉“管理”，不用最近登录过', async () => {
    const rooms = gateway();
    renderGovernance(MEMBER, rooms.value, [], {
      subject: roomWith({
        permissions: { capabilities: ['view', 'speak', 'invite', 'manage'] },
        principalId: MEMBER,
        status: 'joined',
      }),
    });
    const role = await screen.findByRole('combobox', { name: 'Role of Grace' });
    expect(roleChoices(role)).toEqual(['Admin', 'Member']);
    fireEvent.change(role, { target: { value: 'member' } });

    expect(await screen.findByText(/^Stop being an admin\?/u)).toBeVisible();
    expect(rooms.updatePermissions).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Step down' }));
    await waitFor(() => {
      expect(rooms.updatePermissions).toHaveBeenCalledWith(room.catalogId, MEMBER, {
        capabilities: ['view', 'speak', 'invite'],
      });
    });
  });

  it('“更多”里的两个开关：带来的 Agent 能不能说话、能不能邀请人', async () => {
    const rooms = gateway();
    renderGovernance(OWNER, rooms.value);
    fireEvent.click(await screen.findByRole('button', { name: 'More about Grace' }));
    const grace = rowOf('Grace');
    const speak = grace.getByRole('switch', { name: 'Agents they bring can speak' });
    expect(speak).not.toBeChecked();
    fireEvent.click(speak);
    await waitFor(() => {
      expect(rooms.updatePermissions).toHaveBeenCalledWith(room.catalogId, MEMBER, {
        capabilities: ['view', 'speak', 'automate'],
      });
    });
    const invite = grace.getByRole('switch', { name: 'Can invite people' });
    await waitFor(() => {
      expect(invite).toBeEnabled();
    });
    fireEvent.click(invite);
    await waitFor(() => {
      expect(rooms.updatePermissions).toHaveBeenLastCalledWith(room.catalogId, MEMBER, {
        capabilities: ['view', 'speak', 'invite'],
      });
    });
  });

  it('只能看的人：两个开关关着、动不了，改回成员以后他的 Agent 按默认能说话', async () => {
    const rooms = gateway();
    renderGovernance(OWNER, rooms.value, [], {
      subject: roomWith({
        permissions: { capabilities: ['view'] },
        principalId: MEMBER,
        status: 'joined',
      }),
    });
    const role = await screen.findByRole('combobox', { name: 'Role of Grace' });
    expect(role).toHaveValue('viewer');
    fireEvent.click(screen.getByRole('button', { name: 'More about Grace' }));
    for (const name of ['Agents they bring can speak', 'Can invite people']) {
      const toggle = rowOf('Grace').getByRole('switch', { name });
      expect(toggle).not.toBeChecked();
      expect(toggle).toBeDisabled();
    }
    fireEvent.change(role, { target: { value: 'member' } });
    await waitFor(() => {
      expect(rooms.updatePermissions).toHaveBeenCalledWith(room.catalogId, MEMBER, {
        capabilities: ['view', 'speak', 'automate'],
      });
    });
  });

  it('服务器说只有房主能做时把原因写出来', async () => {
    const remove = vi.fn<PrivateRoomGateway['remove']>(() =>
      Promise.resolve(err({ code: 'private_room.owner_only', retryable: false })),
    );
    // 名单还没跟上：这一行在网页看来是成员，服务器上他刚被设成了管理员。
    renderGovernance(BANNED, { ...gateway().value, remove }, [], {
      subject: roomWith({
        permissions: { capabilities: ['view', 'speak', 'manage'] },
        principalId: BANNED,
        status: 'joined',
      }),
    });
    fireEvent.click(await screen.findByRole('button', { name: 'More about Grace' }));
    fireEvent.click(screen.getByRole('button', { name: 'Remove' }));

    expect(await screen.findByText(/^Only the owner can make or remove admins/u)).toBeVisible();
    expect(remove).toHaveBeenCalledWith(room.catalogId, MEMBER);
  });

  it('公开房间：概况照公开设置写，房主看得到在等的申请', async () => {
    renderGovernance(OWNER, gateway().value, [], { publicRoom: true });
    expect(await screen.findByText('Public · Ask to join')).toBeVisible();
    expect(
      screen.getByText('Not encrypted: the people in it and the server can read it.'),
    ).toBeVisible();
    expect(await screen.findByRole('heading', { name: 'Requests to join' })).toBeVisible();
    expect(screen.getByRole('button', { name: 'Approve Lin' })).toBeVisible();
  });

  it('公开房间里不能管理的成员看不到申请', async () => {
    renderGovernance(MEMBER, gateway().value, [], { publicRoom: true });
    expect(await screen.findByText('Public · Ask to join')).toBeVisible();
    expect(screen.queryByRole('heading', { name: 'Requests to join' })).toBeNull();
  });

  it('成员只看到房间名，不能改', () => {
    const rooms = gateway();
    renderGovernance(MEMBER, rooms.value);
    expect(screen.getByText('Architecture room')).toBeVisible();
    expect(screen.queryByLabelText('Room name')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Rename' })).toBeNull();
  });

  it('房主生成口令后看到口令，一键复制给 Agent 的话：和接入对话框里的同一段', async () => {
    const writeText = vi.fn<(text: string) => Promise<void>>(() => Promise.resolve());
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const rooms = gateway();
    renderGovernance(OWNER, rooms.value);
    expect(await screen.findByText(/No code yet/u)).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Create code' }));
    expect(await screen.findByText('K7P3-Q9XW-2DMA')).toBeVisible();
    expect(rooms.generateJoinCode).toHaveBeenCalledWith(room.catalogId);
    expect(rooms.revealJoinCode).not.toHaveBeenCalled();
    expect(screen.getByText(/^Created /u)).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Copy message for the agent' }));
    expect(await screen.findByRole('button', { name: 'Copied' })).toBeVisible();
    const message = writeText.mock.calls[0]?.[0] ?? '';
    expect(message).toContain('with the code K7P3-Q9XW-2DMA');
    expect(message).toContain(`knock with the room number ${room.catalogId}`);
    expect(message).toContain('“Architecture room”');
    expect(message).toContain('/agents.txt');
    // 口令开着时只能换或停用，不再显示生成按钮。
    expect(screen.getByRole('button', { name: 'Replace code' })).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Create code' })).toBeNull();
  });

  it('复制失败时展开给 Agent 的话，让人手动复制', async () => {
    const writeText = vi.fn<(text: string) => Promise<void>>(() =>
      Promise.reject(new Error('denied')),
    );
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const rooms = gateway();
    renderGovernance(OWNER, rooms.value);
    fireEvent.click(await screen.findByRole('button', { name: 'Create code' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Copy message for the agent' }));
    expect(await screen.findByText(/Copy failed/u)).toBeVisible();
    const preview = screen.getByText(/with the code K7P3-Q9XW-2DMA/u);
    expect(preview).toBeVisible();
    expect(preview.closest('details')).toHaveAttribute('open');
  });

  it('已有口令时直接显示口令、能复制；停用后回到生成', async () => {
    const rooms = gateway({ agents: [], joinCode: { createdAtUnixMs: CREATED } });
    renderGovernance(OWNER, rooms.value);
    expect(await screen.findByText(STORED_CODE)).toBeVisible();
    expect(rooms.revealJoinCode).toHaveBeenCalledWith(room.catalogId);
    expect(screen.getByRole('button', { name: 'Copy message for the agent' })).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Turn off code' }));
    await waitFor(() => {
      expect(rooms.disableJoinCode).toHaveBeenCalledWith(room.catalogId);
    });
    expect(await screen.findByRole('button', { name: 'Create code' })).toBeVisible();
    expect(screen.queryByText(STORED_CODE)).toBeNull();
  });

  it('口令看不到时说清楚，要复制就换一个；换完显示新口令', async () => {
    const rooms = gateway({ agents: [], joinCode: { createdAtUnixMs: CREATED } }, [], false);
    renderGovernance(OWNER, rooms.value);
    expect(await screen.findByText(/Code on since .*, but it can’t be shown here/u)).toBeVisible();
    expect(screen.queryByText(STORED_CODE)).toBeNull();
    expect(screen.queryByRole('button', { name: 'Copy message for the agent' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Replace code' }));
    expect(await screen.findByText('K7P3-Q9XW-2DMA')).toBeVisible();
    expect(screen.getByRole('button', { name: 'Copy message for the agent' })).toBeVisible();
  });

  it('列出凭口令进来的 Agent 与主人，移除后不再列出', async () => {
    const rooms = gateway({
      agents: [
        {
          agentId: AGENT,
          displayName: 'Scout',
          joinedAtUnixMs: CREATED,
          ownerDisplayName: 'Mina',
          status: 'joined',
          statusChangedAtUnixMs: CREATED,
        },
        {
          agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e49',
          displayName: 'Gone',
          joinedAtUnixMs: CREATED,
          ownerDisplayName: null,
          status: 'removed',
          statusChangedAtUnixMs: CREATED + 1,
        },
      ],
      joinCode: { createdAtUnixMs: CREATED },
    });
    renderGovernance(OWNER, rooms.value);
    const list = await screen.findByRole('list', { name: 'Agents in this room' });
    expect(list).toHaveTextContent('Scout');
    expect(list).toHaveTextContent('Agent of Mina');
    expect(list).not.toHaveTextContent('Gone');
    fireEvent.click(screen.getByRole('button', { name: 'Remove Scout' }));
    await waitFor(() => {
      expect(rooms.removeCodeAgent).toHaveBeenCalledWith(room.catalogId, AGENT);
    });
    expect(
      await screen.findByText('No agent has come in with the code or been let in yet.'),
    ).toBeVisible();
  });

  it('在敲门的 Agent 列在最前面；放它进来后挪进“进来的 Agent”', async () => {
    const rooms = gateway({ agents: [], joinCode: null }, [
      {
        agentId: AGENT,
        displayName: 'Sol',
        expiresAtUnixMs: CREATED + 3_600_000,
        knockedAtUnixMs: CREATED,
      },
    ]);
    renderGovernance(OWNER, rooms.value);

    const knocks = await screen.findByRole('list', { name: 'Agents knocking' });
    expect(knocks).toHaveTextContent('Sol is knocking');
    expect(
      await screen.findByText('No agent has come in with the code or been let in yet.'),
    ).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Let Sol in' }));

    expect(await screen.findByText('Sol is in')).toBeVisible();
    expect(rooms.admitKnock).toHaveBeenCalledWith(room.catalogId, AGENT);
    expect(await screen.findByRole('list', { name: 'Agents in this room' })).toHaveTextContent(
      'Sol',
    );
    expect(screen.queryByRole('list', { name: 'Agents knocking' })).toBeNull();
  });

  it('口令一节提示网络 Agent 由服务器代收发', async () => {
    renderGovernance(OWNER, gateway().value);
    expect(
      await screen.findByText(/The server sends and receives for them, so once one is here/u),
    ).toBeVisible();
  });

  it('有网络 Agent 凭口令进来时，加密说明写明服务器代它收发', async () => {
    const rooms = gateway({ agents: [codeAgent(AGENT, null)], joinCode: null });
    renderGovernance(OWNER, rooms.value, [AGENT]);
    expect(
      await screen.findByText(
        'End-to-end encrypted. This room has network agents; the server sends and receives for them.',
      ),
    ).toBeVisible();
  });

  it('凭口令进来的只有本机 Agent 时，加密说明不提服务器代收发', async () => {
    const rooms = gateway({ agents: [codeAgent(AGENT, 'Mina')], joinCode: null });
    renderGovernance(OWNER, rooms.value);
    expect(await screen.findByText('Agent of Mina', { exact: false })).toBeVisible();
    expect(screen.getByText('End-to-end encrypted')).toBeVisible();
  });

  it('不能管理房间的成员看不到口令，也不去查', () => {
    const rooms = gateway();
    renderGovernance(MEMBER, rooms.value);
    expect(screen.queryByRole('heading', { name: 'Agent code' })).toBeNull();
    expect(rooms.agentAccess).not.toHaveBeenCalled();
    expect(rooms.agentKnocks).not.toHaveBeenCalled();
  });
});
