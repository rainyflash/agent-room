// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PrivateRoom } from '@/features/private-rooms/domain/private-room';
import type { SubRoom, SubRoomList } from '@/features/private-rooms/domain/sub-rooms';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const services = {
  subRooms: { archive: vi.fn(), revise: vi.fn() },
};

vi.mock('@/app/app-services', () => ({ useAppServices: () => services }));
// 在等的那一块自己有测试（sub-room-waiting.spec.tsx）。
vi.mock('./sub-room-waiting', () => ({ SubRoomWaiting: () => null }));

const { SubRoomSettings, subRoomRevision } = await import('./sub-room-settings');

const PARENT = '0198b601-77a1-7bb8-83eb-a8fe68c97e40';
const WORK = '0198b601-77a1-7bb8-83eb-a8fe68c97e60';
const OPS = '0198b601-77a1-7bb8-83eb-a8fe68c97e61';
const GRACE = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';
const REF = { parentId: PARENT, subRoomId: '0198b601-77a1-7bb8-83eb-a8fe68c97e55' };

function subRoom(extra: Partial<SubRoom> = {}): SubRoom {
  return {
    catalogId: REF.subRoomId,
    name: 'Backup rework',
    description: 'Off-site copies.',
    access: 'request',
    hidden: false,
    createdByPrincipalId: GRACE,
    createdAtUnixMs: 1,
    managerPrincipalIds: [],
    categoryId: OPS,
    joined: true,
    banned: false,
    pendingRequestCount: 2,
    ...extra,
  };
}

function list(room: SubRoom): SubRoomList {
  return {
    parent: { catalogId: PARENT, name: 'Agent Room' },
    canCreate: true,
    categories: [
      { categoryId: WORK, name: 'Work', position: 0 },
      { categoryId: OPS, name: 'Ops', position: 1 },
    ],
    subRooms: [room],
  };
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(() => {
  vi.resetAllMocks();
});

afterEach(() => {
  cleanup();
});

function renderSettings(room: SubRoom, onArchived = vi.fn()) {
  render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={new QueryClient()}>
        <SubRoomSettings list={list(room)} onArchived={onArchived} room={room} />
      </QueryClientProvider>
    </I18nextProvider>,
  );
  return onArchived;
}

describe('只带改了的那几项', () => {
  const draft = {
    name: 'Backup rework',
    purpose: 'Off-site copies.',
    categoryId: OPS,
    access: 'request' as const,
    hidden: false,
  };

  it('什么都没改是 null；名字和用途去掉两头的空白再比', () => {
    expect(subRoomRevision(subRoom(), draft)).toBeNull();
    expect(subRoomRevision(subRoom(), { ...draft, name: ' Backup rework ' })).toBeNull();
    expect(subRoomRevision(subRoom(), { ...draft, purpose: 'Drills too. ' })).toEqual({
      description: 'Drills too.',
    });
  });

  it('挪回“未分类”带 null', () => {
    expect(subRoomRevision(subRoom(), { ...draft, categoryId: '' })).toEqual({ categoryId: null });
    const uncategorized = subRoom();
    delete uncategorized.categoryId;
    expect(subRoomRevision(uncategorized, { ...draft, categoryId: WORK })).toEqual({
      categoryId: WORK,
    });
  });

  it('拆门时不提隐藏，有门时藏起来带上', () => {
    expect(
      subRoomRevision(subRoom({ hidden: true }), { ...draft, access: 'open', hidden: true }),
    ).toEqual({ access: 'open' });
    expect(subRoomRevision(subRoom(), { ...draft, access: 'invite_only', hidden: true })).toEqual({
      access: 'invite_only',
      hidden: true,
    });
  });
});

describe('子房间设置', () => {
  it('改了才能存；只发改了的那几项，存好写“已保存”', async () => {
    services.subRooms.revise.mockResolvedValue(ok({} as PrivateRoom));
    renderSettings(subRoom());
    const save = screen.getByRole('button', { name: 'Save' });
    expect(save).toBeDisabled();

    fireEvent.change(screen.getByRole('textbox', { name: 'Name' }), {
      target: { value: 'Backups' },
    });
    fireEvent.change(screen.getByRole('combobox', { name: 'Category' }), {
      target: { value: WORK },
    });
    fireEvent.click(save);

    await waitFor(() => {
      expect(services.subRooms.revise).toHaveBeenCalledWith(REF, {
        name: 'Backups',
        categoryId: WORK,
      });
    });
  });

  it('名字里不能有斜杠', () => {
    renderSettings(subRoom());
    fireEvent.change(screen.getByRole('textbox', { name: 'Name' }), {
      target: { value: 'Ops/Backups' },
    });
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
  });

  it('换门时说清后果：装门加密，拆门之后进来的看不到以前的，在等的申请作废', () => {
    renderSettings(subRoom());
    fireEvent.click(screen.getByRole('radio', { name: /^Invite only/u }));
    expect(screen.getByText('The 2 requests waiting to join will be dropped.')).toBeVisible();
    fireEvent.click(screen.getByRole('radio', { name: /^Walk in/u }));
    expect(screen.getByText(/People who come in after that can’t read/u)).toBeVisible();
    // 没有门的不能藏。
    const hidden = screen.getByRole('checkbox', { name: /Don’t show it in the sub-room list/u });
    expect(hidden).toBeDisabled();
    expect(hidden).not.toBeChecked();
  });

  it('没有门的装上门：说会加密', () => {
    renderSettings(subRoom({ access: 'open', pendingRequestCount: 0 }));
    fireEvent.click(screen.getByRole('radio', { name: /^Needs approval/u }));
    expect(screen.getByText(/this sub-room is encrypted/u)).toBeVisible();
  });

  it('别人刚换过门：照实说，能从现在的样子重来', async () => {
    services.subRooms.revise.mockResolvedValue(
      err({ code: 'private_room.conflict', retryable: false }),
    );
    renderSettings(subRoom());
    fireEvent.click(screen.getByRole('radio', { name: /^Invite only/u }));
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(await screen.findByText(/Someone just changed who can get in/u)).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Start from what’s there now' }));
    expect(screen.getByRole('radio', { name: /^Needs approval/u })).toBeChecked();
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
  });

  it('归档先在原处确认；归档了交给房间页', async () => {
    services.subRooms.archive.mockResolvedValue(ok(undefined));
    const onArchived = renderSettings(subRoom());
    fireEvent.click(screen.getByRole('button', { name: 'Archive this sub-room' }));
    const confirm = screen.getByRole('group', { name: 'Archive “Backup rework”?' });
    expect(confirm).toHaveTextContent('Anyone who runs it can restore it later.');
    fireEvent.click(screen.getByRole('button', { name: 'Archive' }));
    await waitFor(() => {
      expect(onArchived).toHaveBeenCalled();
    });
    expect(services.subRooms.archive).toHaveBeenCalledWith(REF);
  });
});
