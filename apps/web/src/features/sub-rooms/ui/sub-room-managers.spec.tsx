// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PrivateRoom } from '@/features/private-rooms/domain/private-room';
import type { SubRoom } from '@/features/private-rooms/domain/sub-rooms';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const services = {
  privateRooms: { people: vi.fn() },
  subRooms: { grantManager: vi.fn(), revokeManager: vi.fn() },
};

vi.mock('@/app/app-services', () => ({ useAppServices: () => services }));

const { SubRoomManagers, runsEverySubRoom } = await import('./sub-room-managers');

const PARENT = '0198b601-77a1-7bb8-83eb-a8fe68c97e40';
const ADA = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';
const GRACE = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';
const LIN = '0198b601-77a1-7bb8-83eb-a8fe68c97e45';
const SOL = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const REF = { parentId: PARENT, subRoomId: '0198b601-77a1-7bb8-83eb-a8fe68c97e55' };

const ALL = ['view', 'speak', 'invite', 'manage', 'automate'] as const;
const TALK = ['view', 'speak'] as const;

function member(principalId: string, capabilities: readonly (typeof ALL)[number][]) {
  return { permissions: { capabilities }, principalId, status: 'joined' as const };
}

/** 这一间：Ada 是房主，Grace 是子房间管理员（建的人），Lin、Sol 只是在里面。 */
const MEMBERS: PrivateRoom = {
  catalogId: REF.subRoomId,
  description: '',
  matrixRoomId: '!sub:matrix.test',
  members: [member(ADA, ALL), member(GRACE, ALL), member(LIN, TALK), member(SOL, TALK)],
  name: 'Backup rework',
  ownerPrincipalId: ADA,
  retentionDays: null,
  roomInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e47',
  status: 'active',
  version: 1,
};

/** 父房间：Ada 是房主，Sol 是管理员（还没被这一间的数据算上“管理”）。 */
const PARENT_ROOM: PrivateRoom = {
  ...MEMBERS,
  catalogId: PARENT,
  matrixRoomId: '!parent:matrix.test',
  members: [member(ADA, ALL), member(GRACE, TALK), member(LIN, TALK), member(SOL, ALL)],
  name: 'Agent Room',
};

function subRoom(extra: Partial<SubRoom> = {}): SubRoom {
  return {
    catalogId: REF.subRoomId,
    name: 'Backup rework',
    description: '',
    access: 'request',
    hidden: false,
    createdByPrincipalId: GRACE,
    createdAtUnixMs: 1,
    managerPrincipalIds: [GRACE],
    joined: true,
    banned: false,
    pendingRequestCount: 0,
    ...extra,
  };
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(() => {
  vi.resetAllMocks();
  services.privateRooms.people.mockResolvedValue(
    ok([
      { displayName: 'Ada', principalId: ADA },
      { displayName: 'Grace', principalId: GRACE },
      { displayName: 'Lin', principalId: LIN },
      { displayName: 'Sol', principalId: SOL },
    ]),
  );
  services.subRooms.grantManager.mockResolvedValue(ok(undefined));
  services.subRooms.revokeManager.mockResolvedValue(ok(undefined));
});

afterEach(() => {
  cleanup();
});

function renderManagers({
  onReauthenticate = vi.fn(),
  parent = PARENT_ROOM,
  principalId = ADA,
  recentlyAuthenticated = true,
  room = subRoom(),
}: {
  readonly onReauthenticate?: () => void;
  readonly parent?: PrivateRoom | null;
  readonly principalId?: string;
  readonly recentlyAuthenticated?: boolean;
  readonly room?: SubRoom;
} = {}) {
  render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={new QueryClient()}>
        <SubRoomManagers
          members={MEMBERS}
          onReauthenticate={onReauthenticate}
          parent={parent}
          principalId={principalId}
          recentlyAuthenticated={recentlyAuthenticated}
          room={room}
          subRoomRef={REF}
        />
      </QueryClientProvider>
    </I18nextProvider>,
  );
  return screen.getByRole('region', { name: 'People who run this one' });
}

describe('谁靠整间房管着每一间', () => {
  it('房主、父房间的管理员算；父房间不在手边时，身上有“管理”又不在名单上的算', () => {
    const room = subRoom();
    expect(runsEverySubRoom(ADA, room, MEMBERS, PARENT_ROOM)).toBe(true);
    expect(runsEverySubRoom(SOL, room, MEMBERS, PARENT_ROOM)).toBe(true);
    expect(runsEverySubRoom(GRACE, room, MEMBERS, PARENT_ROOM)).toBe(false);
    expect(runsEverySubRoom(LIN, room, MEMBERS, PARENT_ROOM)).toBe(false);

    expect(runsEverySubRoom(GRACE, room, MEMBERS, null)).toBe(false);
    expect(runsEverySubRoom(GRACE, subRoom({ managerPrincipalIds: [] }), MEMBERS, null)).toBe(true);
  });
});

describe('管这一间的', () => {
  it('一行一个，标出建的人和你；房主和管理员能撤别人', async () => {
    const section = renderManagers({ room: subRoom({ managerPrincipalIds: [ADA, GRACE] }) });

    const ada = (await within(section).findByText('Ada')).closest('li');
    expect(ada).toHaveTextContent('You · Owner');
    // 房主本来就管每一间，名单上那一条撤不撤都一样。
    expect(within(ada as HTMLElement).queryByRole('button')).not.toBeInTheDocument();
    const grace = within(section).getByText('Grace').closest('li');
    expect(grace).toHaveTextContent('Created it');
    fireEvent.click(screen.getByRole('button', { name: 'Stop Grace running this sub-room' }));

    await waitFor(() => {
      expect(services.subRooms.revokeManager).toHaveBeenCalledWith(REF, GRACE);
    });
  });

  it('从这一间里的人里挑一个设成管理员；房主、管理员、已经在管的不在候选里', async () => {
    const section = renderManagers();
    await within(section).findByText('Grace');

    fireEvent.click(screen.getByRole('button', { name: 'Add a manager' }));
    const who = screen.getByRole('combobox', { name: 'Who' });
    expect(who).toHaveFocus();
    expect(
      within(who)
        .getAllByRole('option')
        .map((option) => option.textContent),
    ).toEqual(['Choose someone', 'Lin']);
    expect(screen.getByRole('button', { name: 'Make manager' })).toBeDisabled();
    fireEvent.change(who, { target: { value: LIN } });
    fireEvent.click(screen.getByRole('button', { name: 'Make manager' }));

    await waitFor(() => {
      expect(services.subRooms.grantManager).toHaveBeenCalledWith(REF, LIN);
    });
    await waitFor(() => {
      expect(screen.queryByRole('combobox', { name: 'Who' })).not.toBeInTheDocument();
    });
  });

  it('不算最近登录过：设别人、撤别人前先请他重新登录', async () => {
    const onReauthenticate = vi.fn();
    const section = renderManagers({ onReauthenticate, recentlyAuthenticated: false });
    await within(section).findByText('Grace');

    fireEvent.click(screen.getByRole('button', { name: 'Add a manager' }));
    expect(screen.queryByRole('combobox', { name: 'Who' })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Sign in again' }));
    expect(onReauthenticate).toHaveBeenCalledOnce();

    fireEvent.click(screen.getByRole('button', { name: 'Stop Grace running this sub-room' }));
    expect(services.subRooms.revokeManager).not.toHaveBeenCalled();
  });

  it('子房间管理员：撤不了别人，自己能不当了（先确认，不用重新登录）', async () => {
    const section = renderManagers({
      principalId: GRACE,
      recentlyAuthenticated: false,
      room: subRoom({ managerPrincipalIds: [GRACE, LIN] }),
    });

    const lin = (await within(section).findByText('Lin')).closest('li');
    expect(within(lin as HTMLElement).queryByRole('button')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Step down' }));
    const confirm = screen.getByRole('group', { name: 'Stop running “Backup rework”?' });
    expect(confirm).toHaveTextContent('You stay in this sub-room');
    fireEvent.click(within(confirm).getByRole('button', { name: 'Step down' }));

    await waitFor(() => {
      expect(services.subRooms.revokeManager).toHaveBeenCalledWith(REF, GRACE);
    });
  });

  it('没人单独管：写一句房主和管理员管着；说得清的没办成照实说', async () => {
    services.subRooms.grantManager.mockResolvedValue(
      err({ code: 'sub_room.target_not_inside', retryable: false }),
    );
    renderManagers({ room: subRoom({ managerPrincipalIds: [] }) });

    expect(
      screen.getByText(
        'Nobody runs just this one. The room’s owner and admins run every sub-room.',
      ),
    ).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Add a manager' }));
    fireEvent.change(screen.getByRole('combobox', { name: 'Who' }), { target: { value: GRACE } });
    fireEvent.click(screen.getByRole('button', { name: 'Make manager' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'They’re not in this sub-room any more.',
    );
  });
});
