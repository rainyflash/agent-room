// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { SubRoom } from '@/features/private-rooms/domain/sub-rooms';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const services = {
  privateRooms: { admitKnock: vi.fn(), agentKnocks: vi.fn(), declineKnock: vi.fn() },
  subRooms: { answerJoinRequest: vi.fn(), joinRequests: vi.fn() },
};

vi.mock('@/app/app-services', () => ({ useAppServices: () => services }));

const { SubRoomWaiting } = await import('./sub-room-waiting');

const PARENT = '0198b601-77a1-7bb8-83eb-a8fe68c97e40';
const MIKA = '0198b601-77a1-7bb8-83eb-a8fe68c97e49';
const REF = { parentId: PARENT, subRoomId: '0198b601-77a1-7bb8-83eb-a8fe68c97e55' };

function subRoom(extra: Partial<SubRoom> = {}): SubRoom {
  return {
    catalogId: REF.subRoomId,
    name: 'Backup rework',
    description: '',
    access: 'request',
    hidden: false,
    createdByPrincipalId: PARENT,
    createdAtUnixMs: 1,
    managerPrincipalIds: [],
    joined: true,
    banned: false,
    pendingRequestCount: 1,
    ...extra,
  };
}

const mika = {
  displayName: 'Mika',
  message: 'I run the restore drills.',
  principalId: MIKA,
  requestedAtUnixMs: 1_763_000_000_000,
};

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(() => {
  vi.resetAllMocks();
  services.privateRooms.agentKnocks.mockResolvedValue(ok([]));
  services.subRooms.joinRequests.mockResolvedValue(ok([mika]));
});

afterEach(() => {
  cleanup();
});

function renderWaiting(room: SubRoom) {
  render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={new QueryClient()}>
        <SubRoomWaiting room={room} subRoomRef={REF} />
      </QueryClientProvider>
    </I18nextProvider>,
  );
}

describe('子房间设置里在等的', () => {
  it('列出在等的人和留的话，同意了交给服务器', async () => {
    services.subRooms.answerJoinRequest.mockResolvedValue(ok(undefined));
    renderWaiting(subRoom());

    const section = await screen.findByRole('region', { name: /^Waiting/u });
    const row = (await within(section).findByText('Mika')).closest('li');
    expect(row).not.toBeNull();
    expect(within(section).getByText('I run the restore drills.')).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Approve Mika' }));

    await waitFor(() => {
      expect(services.subRooms.answerJoinRequest).toHaveBeenCalledWith(REF, MIKA, 'approve');
    });
  });

  it('他被这一间封禁了：说先解封', async () => {
    services.subRooms.answerJoinRequest.mockResolvedValue(
      err({ code: 'sub_room.target_banned', retryable: false }),
    );
    renderWaiting(subRoom());

    fireEvent.click(await screen.findByRole('button', { name: 'Approve Mika' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'They’re banned from this sub-room. Unban them first.',
    );
  });

  it('要批准、没人在等：写一句；直接进的没人在等：不出现', async () => {
    services.subRooms.joinRequests.mockResolvedValue(ok([]));
    renderWaiting(subRoom({ pendingRequestCount: 0 }));
    expect(await screen.findByText('Nobody is waiting to join.')).toBeVisible();
    cleanup();

    renderWaiting(subRoom({ access: 'open', pendingRequestCount: 0 }));
    await waitFor(() => {
      expect(services.subRooms.joinRequests).toHaveBeenCalledTimes(2);
    });
    expect(screen.queryByRole('region', { name: /^Waiting/u })).not.toBeInTheDocument();
  });
});
