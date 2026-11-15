// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import {
  permissions,
  type PrivateRoomCapability,
} from '@/features/private-rooms/domain/private-room';
import type {
  RoomRoleChange,
  RoomRoleChangeGateway,
  RoomStanding,
} from '@/features/private-rooms/domain/room-role-changes';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

import { RoomRoleChanges } from './room-role-changes';

const CATALOG = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const ADA = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';
const GRACE = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';
const MALLORY = '0198b601-77a1-7bb8-83eb-a8fe68c97e4a';
const names = new Map([
  [ADA, 'Ada'],
  [GRACE, 'Grace'],
  [MALLORY, 'Mallory'],
]);
const WHEN = Date.UTC(2026, 10, 14, 7, 30);

function standing(
  capabilities: PrivateRoomCapability[],
  status: RoomStanding['status'] = 'joined',
): RoomStanding {
  return { owner: false, permissions: permissions(...capabilities), status };
}

const admin: PrivateRoomCapability[] = ['view', 'speak', 'invite', 'manage'];
const member: PrivateRoomCapability[] = ['view', 'speak'];

/** 新的在前：Grace 自己不当管理员了，Mallory 被封禁，Grace 被设成管理员，再往前是一条没人动手的。 */
const changes: readonly RoomRoleChange[] = [
  {
    actorPrincipalId: GRACE,
    changedAtUnixMs: WHEN,
    id: 4,
    next: standing(member),
    previous: standing(admin),
    targetPrincipalId: GRACE,
  },
  {
    actorPrincipalId: ADA,
    changedAtUnixMs: WHEN - 3_600_000,
    id: 3,
    next: standing([], 'banned'),
    previous: standing(admin),
    targetPrincipalId: MALLORY,
  },
  {
    actorPrincipalId: ADA,
    changedAtUnixMs: WHEN - 7_200_000,
    id: 2,
    next: standing(admin),
    previous: standing(member),
    targetPrincipalId: GRACE,
  },
  {
    changedAtUnixMs: WHEN - 86_400_000,
    id: 1,
    next: standing([...member, 'invite']),
    previous: standing(member),
    targetPrincipalId: '0198b601-77a1-7bb8-83eb-a8fe68c97e4f',
  },
];

function renderChanges(roleChanges: RoomRoleChangeGateway['roleChanges']) {
  return render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={new QueryClient()}>
        <RoomRoleChanges
          catalogId={CATALOG}
          gateway={{ roleChanges }}
          nameOf={(principalId) => names.get(principalId) ?? 'Name not available'}
        />
      </QueryClientProvider>
    </I18nextProvider>,
  );
}

/** jsdom 里点 summary 不发 toggle：照浏览器的样子打开，再发一次。 */
function open(): void {
  const details = screen.getByText('Role changes').closest('details');
  if (details === null) throw new Error('“身份变动”没有收在详情里。');
  details.open = true;
  fireEvent(details, new Event('toggle'));
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en-US']);
});

afterEach(() => {
  cleanup();
});

describe('成员一节最下面的“身份变动”', () => {
  it('平时收着、不去读；点开了才读，一条写说的事和谁动的手、什么时候', async () => {
    const roleChanges = vi.fn<RoomRoleChangeGateway['roleChanges']>(() =>
      Promise.resolve(ok(changes)),
    );
    renderChanges(roleChanges);
    expect(roleChanges).not.toHaveBeenCalled();

    open();

    const items = within(await screen.findByRole('list')).getAllByRole('listitem');
    expect(roleChanges).toHaveBeenCalledWith(CATALOG);
    expect(items.map((item) => item.querySelector('p')?.textContent)).toEqual([
      'Grace went from Admin to Member',
      'Mallory (Admin) was banned',
      'Grace went from Member to Admin',
      'Name not available can now invite people',
    ]);
    const time = new Intl.DateTimeFormat('en-US', { dateStyle: 'medium', timeStyle: 'short' });
    expect(items.map((item) => item.querySelector('small')?.textContent)).toEqual([
      `By Grace · ${time.format(WHEN)}`,
      `By Ada · ${time.format(WHEN - 3_600_000)}`,
      `By Ada · ${time.format(WHEN - 7_200_000)}`,
      // 不是谁动的手：只写时间。
      time.format(WHEN - 86_400_000),
    ]);
    expect(screen.queryByText(/latest 100/u)).toBeNull();
  });

  it('子房间里的写着是哪一间，那里的“管理”叫子房间管理员', async () => {
    const inRelease: RoomRoleChange = {
      actorPrincipalId: ADA,
      changedAtUnixMs: WHEN,
      id: 9,
      next: standing(['view', 'speak', 'manage']),
      previous: standing(member),
      subRoom: { catalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e49', name: 'Release' },
      targetPrincipalId: GRACE,
    };
    renderChanges(() => Promise.resolve(ok([inRelease])));
    open();

    const item = within(await screen.findByRole('list')).getByRole('listitem');
    expect(item.querySelector('p')?.textContent).toBe('Grace went from Member to Sub-room admin');
    const time = new Intl.DateTimeFormat('en-US', { dateStyle: 'medium', timeStyle: 'short' });
    expect(item.querySelector('small')?.textContent).toBe(
      `By Ada · ${time.format(WHEN)} · in the sub-room Release`,
    );
  });

  it('一条也没有时说一句；满一百条时说更早的不在这里', async () => {
    const first = renderChanges(() => Promise.resolve(ok([])));
    open();
    expect(await screen.findByText('No role changes yet.')).toBeInTheDocument();
    first.unmount();

    const base = changes[2];
    if (base === undefined) throw new Error('夹具少了一条。');
    const hundred = Array.from({ length: 100 }, (_, index) => ({ ...base, id: 200 - index }));
    renderChanges(() => Promise.resolve(ok(hundred)));
    open();
    expect(
      await screen.findByText('These are the latest 100. Older ones aren’t shown here.'),
    ).toBeInTheDocument();
    expect(screen.getAllByRole('listitem')).toHaveLength(100);
  });

  it('读不到时说没做成，能再试一次', async () => {
    const roleChanges = vi
      .fn<RoomRoleChangeGateway['roleChanges']>()
      .mockResolvedValueOnce(
        err({ code: 'room_role_changes.dependency_unavailable', retryable: true }),
      )
      .mockResolvedValue(ok(changes));
    renderChanges(roleChanges);
    open();

    expect(await screen.findByText('room_role_changes.dependency_unavailable')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }));

    expect(await screen.findByText('Mallory (Admin) was banned')).toBeInTheDocument();
    expect(roleChanges).toHaveBeenCalledTimes(2);
  });
});
