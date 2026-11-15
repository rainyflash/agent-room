// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  SubRoom,
  SubRoomCategory,
  SubRoomList,
  SubRoomPolicy,
} from '@/features/private-rooms/domain/sub-rooms';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const services = {
  subRooms: {
    changePolicy: vi.fn(),
    createCategory: vi.fn(),
    deleteCategory: vi.fn(),
    policy: vi.fn(),
    reviseCategory: vi.fn(),
    subRooms: vi.fn(),
  },
};

vi.mock('@/app/app-services', () => ({ useAppServices: () => services }));

const { RoomSubRoomsSettings } = await import('./room-sub-rooms-settings');

const PARENT = '0198b601-77a1-7bb8-83eb-a8fe68c97e40';
const WORK = '0198b601-77a1-7bb8-83eb-a8fe68c97e60';
const OPS = '0198b601-77a1-7bb8-83eb-a8fe68c97e61';
const UUID_V7 = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u;

const CATEGORIES: readonly SubRoomCategory[] = [
  { categoryId: OPS, name: 'Ops', position: 1 },
  { categoryId: WORK, name: 'Work', position: 0 },
];

function subRoom(id: string, categoryId?: string): SubRoom {
  return {
    catalogId: `0198b601-77a1-7bb8-83eb-a8fe68c97e${id}`,
    name: `Room ${id}`,
    description: '',
    access: 'open',
    hidden: false,
    ...(categoryId === undefined ? {} : { categoryId }),
    createdByPrincipalId: PARENT,
    createdAtUnixMs: 1,
    managerPrincipalIds: [],
    joined: true,
    banned: false,
  };
}

const LIST: SubRoomList = {
  parent: { catalogId: PARENT, name: 'Agent Room' },
  canCreate: true,
  categories: [...CATEGORIES],
  subRooms: [subRoom('51', WORK), subRoom('52', WORK), subRoom('53')],
};

const POLICY: SubRoomPolicy = {
  creators: 'members',
  agentsMayCreate: true,
  autoArchive: true,
  canChange: true,
};

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(() => {
  vi.resetAllMocks();
  services.subRooms.policy.mockResolvedValue(ok(POLICY));
  services.subRooms.subRooms.mockResolvedValue(ok(LIST));
});

afterEach(() => {
  cleanup();
});

function renderSettings() {
  render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={new QueryClient()}>
        <RoomSubRoomsSettings parentId={PARENT} />
      </QueryClientProvider>
    </I18nextProvider>,
  );
}

async function categoryRows() {
  const section = await screen.findByRole('region', { name: /^Categories/u });
  return within(section)
    .getAllByRole('listitem')
    .map((row) => row.querySelector('strong')?.textContent);
}

describe('房间设置里的“子房间”一节', () => {
  it('三项设置改了马上存，发的是改完的整份', async () => {
    // 存好以后会再问一遍：服务器那边已经是改过的了。
    const admins = { ...POLICY, creators: 'admins' } as const;
    services.subRooms.changePolicy.mockResolvedValue(ok(admins));
    renderSettings();

    const adminsOnly = await screen.findByRole('radio', { name: 'Admins only' });
    services.subRooms.policy.mockResolvedValue(ok(admins));
    fireEvent.click(adminsOnly);
    await waitFor(() => {
      expect(services.subRooms.changePolicy).toHaveBeenCalledWith(PARENT, {
        creators: 'admins',
        agentsMayCreate: true,
        autoArchive: true,
      });
    });
    await waitFor(() => {
      expect(screen.getByRole('radio', { name: 'Admins only' })).toBeChecked();
    });

    const kept = { ...admins, autoArchive: false };
    services.subRooms.changePolicy.mockResolvedValue(ok(kept));
    services.subRooms.policy.mockResolvedValue(ok(kept));
    fireEvent.click(screen.getByRole('switch', { name: /^Archive after 30 days/u }));
    await waitFor(() => {
      expect(services.subRooms.changePolicy).toHaveBeenLastCalledWith(PARENT, {
        creators: 'admins',
        agentsMayCreate: true,
        autoArchive: false,
      });
    });
  });

  it('不是房主：三项只看、改不了，写一句只有房主能改', async () => {
    services.subRooms.policy.mockResolvedValue(ok({ ...POLICY, canChange: false }));
    renderSettings();

    expect(await screen.findByText('Only the room’s owner can change these.')).toBeVisible();
    expect(screen.queryByRole('radio', { name: 'Admins only' })).not.toBeInTheDocument();
    expect(screen.getByText('All members')).toBeVisible();
    expect(screen.getByRole('switch', { name: /^Agents can create/u })).toBeDisabled();
  });

  it('分类按先后列，写着几间，最后是“未分类”', async () => {
    renderSettings();

    expect(await categoryRows()).toEqual(['Work', 'Ops', 'Uncategorized']);
    const section = screen.getByRole('region', { name: /^Categories/u });
    expect(section).toHaveTextContent('Work2 sub-rooms');
    expect(section).toHaveTextContent('Ops0 sub-rooms');
    expect(section).toHaveTextContent('Uncategorized1 sub-room');
    expect(screen.getByRole('button', { name: 'Move Work up' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Move Ops down' })).toBeDisabled();
  });

  it('新建分类带上网页定好的分类号；名字撞了照实说', async () => {
    services.subRooms.createCategory.mockResolvedValue(
      err({ code: 'sub_room.category_name_taken', retryable: false }),
    );
    renderSettings();

    fireEvent.click(await screen.findByRole('button', { name: 'New category' }));
    const name = screen.getByRole('textbox', { name: 'Name of the new category' });
    expect(name).toHaveFocus();
    fireEvent.change(name, { target: { value: ' ops ' } });
    fireEvent.click(screen.getByRole('button', { name: 'Create' }));

    await waitFor(() => {
      expect(services.subRooms.createCategory).toHaveBeenCalledWith(PARENT, {
        categoryId: expect.stringMatching(UUID_V7) as string,
        name: 'ops',
      });
    });
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'There’s already a category with this name.',
    );
  });

  it('挪、改名、删：里面有子房间的先问一句，空的直接删', async () => {
    services.subRooms.reviseCategory.mockResolvedValue(ok(CATEGORIES));
    services.subRooms.deleteCategory.mockResolvedValue(ok(CATEGORIES));
    renderSettings();

    fireEvent.click(await screen.findByRole('button', { name: 'Move Ops up' }));
    await waitFor(() => {
      expect(services.subRooms.reviseCategory).toHaveBeenCalledWith(PARENT, OPS, { position: 0 });
    });

    fireEvent.click(await screen.findByRole('button', { name: 'Rename Work' }));
    const rename = screen.getByRole('textbox', { name: 'Rename Work' });
    fireEvent.change(rename, { target: { value: 'Shipping' } });
    fireEvent.click(screen.getByRole('button', { name: 'Rename' }));
    await waitFor(() => {
      expect(services.subRooms.reviseCategory).toHaveBeenLastCalledWith(PARENT, WORK, {
        name: 'Shipping',
      });
    });

    fireEvent.click(await screen.findByRole('button', { name: 'Delete Work' }));
    const confirm = screen.getByRole('group', { name: 'Delete “Work”?' });
    expect(confirm).toHaveTextContent('Its 2 sub-rooms go back to Uncategorized.');
    expect(services.subRooms.deleteCategory).not.toHaveBeenCalled();
    fireEvent.click(within(confirm).getByRole('button', { name: 'Delete' }));
    await waitFor(() => {
      expect(services.subRooms.deleteCategory).toHaveBeenCalledWith(PARENT, WORK);
    });

    fireEvent.click(screen.getByRole('button', { name: 'Delete Ops' }));
    await waitFor(() => {
      expect(services.subRooms.deleteCategory).toHaveBeenLastCalledWith(PARENT, OPS);
    });
  });
});
