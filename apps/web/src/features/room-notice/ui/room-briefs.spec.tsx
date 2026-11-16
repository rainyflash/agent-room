// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it } from 'vitest';

import { AppServicesProvider, type AppServices } from '@/app/app-services';
import type { LobbyGateway, LobbyRoom } from '@/features/lobby/domain/lobby';
import type { RoomNotice } from '@/features/room-notice/domain/room-notice';
import { RoomBriefs, type RoomBriefsProps } from '@/features/room-notice/ui/room-briefs';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const PARENT_ID = '!parent:matrix.test';
const SUB_ID = '!sub:matrix.test';
const SELF = '@mina:matrix.test';
const GRACE = '@grace:matrix.test';

const parentNotice: RoomNotice = {
  text: 'This project ships the app.\nSay what you will do before you do it.',
  updatedAtUnixMs: Date.parse('2026-11-02T08:00:00.000Z'),
  updatedBy: GRACE,
};
const ownNotice: RoomNotice = {
  text: 'Run the release checks on the Windows machine.',
  updatedAtUnixMs: Date.parse('2026-11-03T08:00:00.000Z'),
  updatedBy: SELF,
};

/** 只认父房间那一间的 Matrix 状态；`change` 换掉父房间的须知并通知订阅的人。 */
function parentLobby(initial: RoomNotice | undefined) {
  let notice = initial;
  let reads = 0;
  const listeners = new Set<() => void>();
  const gateway: LobbyGateway = {
    read: (roomId) => {
      reads += 1;
      if (roomId !== PARENT_ID) return err({ code: 'lobby.room_not_joined', retryable: true });
      return ok({
        roomId,
        memberNames: [{ displayName: 'Grace', matrixUserId: GRACE }],
        ...(notice === undefined ? {} : { notice }),
      } as unknown as LobbyRoom);
    },
    subscribe: (_roomId, listener) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
  return {
    gateway,
    reads: () => reads,
    change: (next: RoomNotice | undefined) => {
      notice = next;
      for (const listener of listeners) listener();
    },
    ping: () => {
      for (const listener of listeners) listener();
    },
  };
}

function renderBriefs(lobby: LobbyGateway, props: Partial<RoomBriefsProps> = {}) {
  const services = { lobby } as unknown as AppServices;
  return render(
    <I18nextProvider i18n={i18n}>
      <AppServicesProvider services={services}>
        <RoomBriefs
          parent={{ matrixRoomId: PARENT_ID, name: 'Agent Room' }}
          room={{ roomId: SUB_ID, notice: ownNotice }}
          selfMatrixUserId={SELF}
          {...props}
        />
      </AppServicesProvider>
    </I18nextProvider>,
  );
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(async () => {
  window.localStorage.clear();
  await i18n.changeLanguage('en');
});

afterEach(cleanup);

describe('子房间里父房间的须知', () => {
  it('先写父房间的、标着来自哪间房，再写这一间的；父房间那条没看过亮点，在这里不给改', () => {
    const parent = parentLobby(parentNotice);
    renderBriefs(parent.gateway, { onEdit: () => undefined });

    const regions = screen.getAllByRole('region');
    expect(regions.map((region) => region.getAttribute('aria-label'))).toEqual([
      'Brief from Agent Room',
      'This sub-room’s brief',
    ]);
    const [fromParent, own] = regions;
    if (fromParent === undefined || own === undefined) throw new Error('少了一条须知');
    expect(fromParent).toHaveTextContent('From Agent Room');
    expect(fromParent).toHaveTextContent('This project ships the app.');
    expect(within(fromParent).getByText('Changed since you last looked')).toBeInTheDocument();
    expect(within(fromParent).queryByRole('button', { name: 'Edit brief' })).toBeNull();
    expect(within(own).getByRole('button', { name: 'Edit brief' })).toBeVisible();

    fireEvent.click(within(fromParent).getByRole('button', { name: /Show the whole brief/u }));
    expect(within(fromParent).getByText(/^Changed by Grace · /u)).toBeVisible();
    expect(within(fromParent).queryByText('Changed since you last looked')).toBeNull();
  });

  it('父房间没有须知：只写这一间的，叫“须知”；两边都没有就什么都不写', () => {
    const parent = parentLobby(undefined);
    renderBriefs(parent.gateway);
    expect(
      screen.getAllByRole('region').map((region) => region.getAttribute('aria-label')),
    ).toEqual(['Room brief']);

    cleanup();
    const { container } = renderBriefs(parent.gateway, { room: { roomId: SUB_ID } });
    expect(container).toBeEmptyDOMElement();
  });

  it('子房间自己没有：只写父房间的那条', () => {
    renderBriefs(parentLobby(parentNotice).gateway, { room: { roomId: SUB_ID } });
    expect(
      screen.getAllByRole('region').map((region) => region.getAttribute('aria-label')),
    ).toEqual(['Brief from Agent Room']);
  });

  it('父房间改了须知跟着变；父房间别的动静不让它换一份', () => {
    const parent = parentLobby(parentNotice);
    renderBriefs(parent.gateway, { room: { roomId: SUB_ID } });
    const region = screen.getByRole('region', { name: 'Brief from Agent Room' });
    const before = within(region).getByText(/This project ships the app\./u);

    act(() => {
      parent.ping();
    });
    expect(within(region).getByText(/This project ships the app\./u)).toBe(before);

    act(() => {
      parent.change({
        ...parentNotice,
        text: 'Release on Thursdays only.',
        updatedAtUnixMs: parentNotice.updatedAtUnixMs + 60_000,
      });
    });
    expect(screen.getByRole('region', { name: 'Brief from Agent Room' })).toHaveTextContent(
      'Release on Thursdays only.',
    );

    act(() => {
      parent.change(undefined);
    });
    expect(screen.queryByRole('region')).toBeNull();
  });

  it('不是子房间，或者父房间在 Matrix 上是哪一间还没问到：不去读', () => {
    const parent = parentLobby(parentNotice);
    renderBriefs(parent.gateway, { parent: null });
    expect(
      screen.getAllByRole('region').map((region) => region.getAttribute('aria-label')),
    ).toEqual(['Room brief']);

    cleanup();
    renderBriefs(parent.gateway, { parent: { matrixRoomId: null, name: 'Agent Room' } });
    expect(screen.getByRole('region', { name: 'Room brief' })).toBeVisible();
    expect(parent.reads()).toBe(0);
  });
});
