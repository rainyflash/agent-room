// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { LobbyAgent } from '@/features/lobby/domain/lobby';
import type { PrivateRoom } from '@/features/private-rooms/domain/private-room';
import type { SubRoomList } from '@/features/private-rooms/domain/sub-rooms';
import type { OwnedAgent } from '@/features/workspace/domain/agent-directory';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const commands = { enter: vi.fn(), decline: vi.fn(), create: vi.fn(), archive: vi.fn() };
const services = {
  agentDirectory: { bringAgentIntoRoom: vi.fn(), listOwnedAgents: vi.fn() },
  subRooms: { create: vi.fn() },
};

vi.mock('@/app/app-services', () => ({ useAppServices: () => services }));
vi.mock('@/features/room-directory/ui/use-rooms', () => ({ useRoomCommands: () => commands }));

const { NewSubRoomDialog } = await import('./new-sub-room-dialog');

const PARENT = '0198b601-77a1-7bb8-83eb-a8fe68c97e40';
const WORK = '0198b601-77a1-7bb8-83eb-a8fe68c97e60';
const OPS = '0198b601-77a1-7bb8-83eb-a8fe68c97e61';
const MINE = '0198b601-77a1-7bb8-83eb-a8fe68c97e81';
const ALSO_MINE = '0198b601-77a1-7bb8-83eb-a8fe68c97e82';
const ELSEWHERE = '0198b601-77a1-7bb8-83eb-a8fe68c97e83';
const GRACES = '0198b601-77a1-7bb8-83eb-a8fe68c97e84';

const LIST: SubRoomList = {
  parent: { catalogId: PARENT, name: 'Agent Room' },
  canCreate: true,
  categories: [
    { categoryId: OPS, name: 'Ops', position: 1 },
    { categoryId: WORK, name: 'Work', position: 0 },
  ],
  subRooms: [],
};

function owned(agentId: string, displayName: string): OwnedAgent {
  return {
    agentId,
    avatarContentId: null,
    description: '',
    displayName,
    matrixUserId: `@${displayName}:matrix.test`,
    registeredAtUnixMs: 1,
    slug: displayName,
    visibility: 'private',
  };
}

function inRoom(agentId: string, displayName: string): LobbyAgent {
  return { agentId, displayName } as LobbyAgent;
}

const ROOM_AGENTS = [
  inRoom(MINE, 'reviewer #2316'),
  inRoom(GRACES, 'grace-bot #1101'),
  inRoom(ALSO_MINE, 'builder #4410'),
  // 同一个 Agent 的两个连接在名单上是两行，只带一次。
  inRoom(MINE, 'reviewer #2316'),
];

const CREATED = {
  catalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e90',
  matrixRoomId: '!backups:matrix.test',
  name: 'Backups',
} as unknown as PrivateRoom;

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(() => {
  vi.resetAllMocks();
  services.agentDirectory.listOwnedAgents.mockResolvedValue(
    ok([
      owned(MINE, 'reviewer #2316'),
      owned(ALSO_MINE, 'builder #4410'),
      owned(ELSEWHERE, 'off-room #9000'),
    ]),
  );
  services.agentDirectory.bringAgentIntoRoom.mockResolvedValue(ok(1));
  services.subRooms.create.mockResolvedValue(ok(CREATED));
  commands.enter.mockResolvedValue(ok(CREATED));
});

afterEach(() => {
  cleanup();
});

function renderDialog() {
  const onClose = vi.fn();
  render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={new QueryClient()}>
        <NewSubRoomDialog list={LIST} onClose={onClose} roomAgents={ROOM_AGENTS} />
      </QueryClientProvider>
    </I18nextProvider>,
  );
  return { onClose };
}

function name(value: string): void {
  fireEvent.change(screen.getByRole('textbox', { name: 'Name' }), { target: { value } });
}

function submitButton(): HTMLElement {
  return screen.getByRole('button', { name: /Create and go in|Try again/u });
}

describe('新建子房间', () => {
  it('填好名字、用途、分类和门，建好、带上你在这间房里的 Agent、再进去', async () => {
    const { onClose } = renderDialog();

    expect(screen.getByRole('dialog', { name: 'New sub-room' })).toHaveAccessibleDescription(
      'Under Agent Room',
    );
    expect(submitButton()).toBeDisabled();
    // 分类按先后，“未分类”在最前。
    expect(
      within(screen.getByRole('combobox', { name: 'Category' }))
        .getAllByRole('option')
        .map((option) => option.textContent),
    ).toEqual(['Uncategorized', 'Work', 'Ops']);
    expect(screen.getByRole('radio', { name: /^Walk in/u })).toBeChecked();
    // 只列你的、此刻在这间房里的，同一个只列一次。
    const bring = await screen.findByRole('checkbox', { name: /Bring my agents/u });
    expect(bring).toBeChecked();
    expect(bring.closest('label')).toHaveTextContent('reviewer #2316, builder #4410');

    name('  Backups ');
    fireEvent.change(screen.getByRole('textbox', { name: /Purpose/u }), {
      target: { value: ' Where backups go. ' },
    });
    fireEvent.change(screen.getByRole('combobox', { name: 'Category' }), {
      target: { value: OPS },
    });
    fireEvent.click(screen.getByRole('radio', { name: /^Needs approval/u }));
    fireEvent.click(submitButton());

    await waitFor(() => {
      expect(onClose).toHaveBeenCalledOnce();
    });
    const [parentId, input] = services.subRooms.create.mock.calls[0] as [
      string,
      { catalogId: string },
    ];
    expect(parentId).toBe(PARENT);
    expect(input).toEqual({
      access: 'request',
      catalogId: input.catalogId,
      categoryId: OPS,
      description: 'Where backups go.',
      name: 'Backups',
    });
    expect(input.catalogId).toMatch(/^[0-9a-f-]{36}$/u);
    expect(services.agentDirectory.bringAgentIntoRoom.mock.calls).toEqual([
      [MINE, input.catalogId],
      [ALSO_MINE, input.catalogId],
    ]);
    expect(commands.enter).toHaveBeenCalledWith(CREATED, false);
  });

  it('不勾“带进来”就不带', async () => {
    renderDialog();
    fireEvent.click(await screen.findByRole('checkbox', { name: /Bring my agents/u }));
    name('Backups');
    fireEvent.click(submitButton());

    await waitFor(() => {
      expect(commands.enter).toHaveBeenCalledOnce();
    });
    expect(services.agentDirectory.bringAgentIntoRoom).not.toHaveBeenCalled();
  });

  it('名字里不能有“/”', () => {
    renderDialog();
    name('ops/backups');

    expect(screen.getByRole('textbox', { name: 'Name' })).toHaveAccessibleDescription(
      'Names can’t contain “/”.',
    );
    expect(submitButton()).toBeDisabled();
  });

  it('名字撞了：说清楚，改了名字再建是一个新请求', async () => {
    services.subRooms.create.mockResolvedValueOnce(
      err({ code: 'sub_room.name_taken', retryable: false }),
    );
    renderDialog();
    name('Backups');
    fireEvent.click(submitButton());

    expect(await screen.findByText('There’s already a sub-room with this name.')).toBeVisible();
    expect(screen.getByRole('textbox', { name: 'Name' })).toBeEnabled();
    // 一改名字，旧的那句就收起来。
    name('Backups 2');
    expect(
      screen.queryByText('There’s already a sub-room with this name.'),
    ).not.toBeInTheDocument();
    fireEvent.click(submitButton());

    await waitFor(() => {
      expect(commands.enter).toHaveBeenCalledOnce();
    });
    const ids = services.subRooms.create.mock.calls.map(
      ([, input]) => (input as { catalogId: string }).catalogId,
    );
    expect(ids).toHaveLength(2);
    expect(ids[0]).not.toBe(ids[1]);
  });

  it('建好了、没进去：照实说；再试是同一个请求，服务器回同一间', async () => {
    commands.enter.mockResolvedValueOnce(
      err({ code: 'private_room.matrix_join_failed', retryable: true }),
    );
    const { onClose } = renderDialog();
    name('Backups');
    fireEvent.click(submitButton());

    expect(
      await screen.findByText('The sub-room was created, but you couldn’t go in yet.'),
    ).toBeVisible();
    expect(screen.getByText('private_room.matrix_join_failed')).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Name' })).toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }));

    await waitFor(() => {
      expect(onClose).toHaveBeenCalledOnce();
    });
    const ids = services.subRooms.create.mock.calls.map(
      ([, input]) => (input as { catalogId: string }).catalogId,
    );
    expect(ids).toHaveLength(2);
    expect(ids[0]).toBe(ids[1]);
  });
});
