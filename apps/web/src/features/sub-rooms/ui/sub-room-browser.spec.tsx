// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PrivateRoom } from '@/features/private-rooms/domain/private-room';
import type { SubRoom, SubRoomList } from '@/features/private-rooms/domain/sub-rooms';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const commands = { enter: vi.fn(), decline: vi.fn(), create: vi.fn(), archive: vi.fn() };
const services = {
  privateRooms: { inspect: vi.fn(), people: vi.fn() },
  subRooms: {
    join: vi.fn(),
    requestToJoin: vi.fn(),
    restore: vi.fn(),
    subRooms: vi.fn(),
    withdrawJoinRequest: vi.fn(),
  },
};

vi.mock('@/app/app-services', () => ({ useAppServices: () => services }));
vi.mock('@/features/room-directory/ui/use-rooms', () => ({ useRoomCommands: () => commands }));
vi.mock('@/features/inbox/ui/room-unread-mark', () => ({ useCatalogUnread: () => undefined }));
vi.mock('@tanstack/react-router', () => ({
  Link: ({
    children,
    params,
    search,
    to,
    ...rest
  }: {
    readonly children: ReactNode;
    readonly params: { readonly catalogId: string };
    readonly search?: { readonly view?: string };
    readonly to: string;
  }) => (
    <a
      href={`${to.replace('$catalogId', params.catalogId)}${search?.view === undefined ? '' : `?view=${search.view}`}`}
      {...rest}
    >
      {children}
    </a>
  ),
}));

const { SubRoomBrowser } = await import('./sub-room-browser');

const PARENT = '0198b601-77a1-7bb8-83eb-a8fe68c97e40';
const WORK = '0198b601-77a1-7bb8-83eb-a8fe68c97e60';
const ME = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';
const GRACE = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';

function subRoom(id: string, name: string, extra: Partial<SubRoom> = {}): SubRoom {
  return {
    catalogId: `0198b601-77a1-7bb8-83eb-a8fe68c97e${id}`,
    name,
    description: '',
    access: 'open',
    hidden: false,
    createdByPrincipalId: ME,
    createdAtUnixMs: 1,
    managerPrincipalIds: [],
    joined: false,
    banned: false,
    ...extra,
  };
}

const RELEASE = subRoom('51', 'Release', {
  categoryId: WORK,
  description: 'What ships',
  joined: true,
  memberCount: 3,
  agentCount: 4,
});
const DRAFTS = subRoom('52', 'Drafts', { categoryId: WORK, memberCount: 2, agentCount: 1 });
const ALERTS = subRoom('53', 'Alerts', { access: 'request' });
const SECURITY = subRoom('54', 'Security', { access: 'invite_only' });

const LIST: SubRoomList = {
  parent: { catalogId: PARENT, name: 'Agent Room' },
  canCreate: true,
  categories: [{ categoryId: WORK, name: 'Work', position: 0 }],
  subRooms: [RELEASE, DRAFTS, ALERTS, SECURITY],
};

const ARCHIVED: SubRoomList = {
  ...LIST,
  subRooms: [
    subRoom('71', 'Alpha 94', {
      categoryId: WORK,
      joined: true,
      archivedAtUnixMs: Date.UTC(2026, 10, 9),
      archivedByPrincipalId: GRACE,
      archivedReason: 'manual',
      canRestore: true,
    }),
    subRoom('72', 'Mine', {
      archivedAtUnixMs: Date.UTC(2026, 10, 8),
      archivedByPrincipalId: ME,
      archivedReason: 'manual',
      canRestore: true,
    }),
    subRoom('73', 'Quiet', {
      archivedAtUnixMs: Date.UTC(2026, 10, 1),
      archivedReason: 'inactive',
      canRestore: false,
    }),
  ],
};

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(() => {
  vi.resetAllMocks();
  services.subRooms.subRooms.mockResolvedValue(ok(ARCHIVED));
  services.privateRooms.people.mockResolvedValue(
    ok([
      { displayName: 'Ada', principalId: ME },
      { displayName: 'Grace', principalId: GRACE },
    ]),
  );
});

afterEach(() => {
  cleanup();
});

function renderBrowser() {
  render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={new QueryClient()}>
        <SubRoomBrowser list={LIST} principalId={ME} />
      </QueryClientProvider>
    </I18nextProvider>,
  );
}

function row(name: string): HTMLElement {
  const cell = screen.getByText(name, { selector: '.sub-room-browser__name' });
  const found = cell.closest<HTMLElement>('.sub-room-browser__row');
  if (found === null) throw new Error(`没有 ${name} 那一行`);
  return found;
}

describe('浏览子房间', () => {
  it('按分类排，“未分类”在最后；每一行写人数或者门，动作照你的处境', async () => {
    renderBrowser();

    expect(screen.getAllByRole('heading', { level: 3 }).map((h) => h.textContent)).toEqual([
      'Work2',
      'Uncategorized2',
    ]);
    expect(within(row('Release')).getByRole('link', { name: 'Release' })).toHaveAttribute(
      'href',
      `/lobby/${RELEASE.catalogId}`,
    );
    expect(row('Release')).toHaveTextContent('3 people · 4 agents');
    expect(row('Release')).toHaveTextContent('Joined');
    expect(within(row('Drafts')).getByRole('button', { name: 'Join Drafts' })).toBeVisible();
    expect(row('Alerts')).toHaveTextContent('Needs approval');
    expect(within(row('Security')).queryByRole('button')).not.toBeInTheDocument();
    expect(row('Security')).toHaveTextContent('Invite only');
    // 标签上写着几间；归档的问回来以后才有数。
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Archived3' })).toBeVisible();
    });
    expect(screen.getByRole('button', { name: 'All4' })).toHaveAttribute('aria-pressed', 'true');
  });

  it('搜名字和用途；“我在的”只留你在的', () => {
    renderBrowser();

    fireEvent.change(screen.getByRole('searchbox', { name: 'Search sub-rooms' }), {
      target: { value: 'ships' },
    });
    expect(screen.getAllByText(/./u, { selector: '.sub-room-browser__name' })).toHaveLength(1);
    fireEvent.change(screen.getByRole('searchbox', { name: 'Search sub-rooms' }), {
      target: { value: 'nothing here' },
    });
    expect(
      screen.getByText('No sub-room has “nothing here” in its name or purpose.'),
    ).toBeVisible();

    fireEvent.change(screen.getByRole('searchbox', { name: 'Search sub-rooms' }), {
      target: { value: '' },
    });
    fireEvent.click(screen.getByRole('button', { name: /^Joined/u }));
    expect(
      screen.getAllByText(/./u, { selector: '.sub-room-browser__name' }).map((n) => n.textContent),
    ).toEqual(['Release']);
  });

  it('加入：服务器记成已加入以后照进私人房间那样进去', async () => {
    const joined = { catalogId: DRAFTS.catalogId } as unknown as PrivateRoom;
    services.subRooms.join.mockResolvedValue(ok(joined));
    commands.enter.mockResolvedValue(ok(joined));
    renderBrowser();

    fireEvent.click(screen.getByRole('button', { name: 'Join Drafts' }));
    await waitFor(() => {
      expect(commands.enter).toHaveBeenCalledWith(joined, false);
    });
    expect(services.subRooms.join).toHaveBeenCalledWith({
      parentId: PARENT,
      subRoomId: DRAFTS.catalogId,
    });
  });

  it('申请：在那一行下面留句话发出去；没办成的说出来', async () => {
    services.subRooms.requestToJoin.mockResolvedValue(
      err({ code: 'sub_room.recently_rejected', retryable: false }),
    );
    renderBrowser();

    fireEvent.click(screen.getByRole('button', { name: 'Ask to join Alerts' }));
    const form = screen.getByRole('form', { name: 'Ask to join Alerts' });
    const note = within(form).getByRole('textbox');
    expect(note).toHaveFocus();
    fireEvent.change(note, { target: { value: '  on call  ' } });
    fireEvent.click(within(form).getByRole('button', { name: 'Send request' }));

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent(
        'Your request was just turned down. Try again later.',
      );
    });
    expect(services.subRooms.requestToJoin).toHaveBeenCalledWith(
      { parentId: PARENT, subRoomId: ALERTS.catalogId },
      'on call',
    );
  });

  it('已归档：写原来的分类、谁什么时候归档的；原来在里面的能打开，管得了的能恢复', async () => {
    services.subRooms.restore.mockResolvedValue(ok(undefined));
    renderBrowser();

    fireEvent.click(await screen.findByRole('button', { name: 'Archived3' }));
    await waitFor(() => {
      expect(row('Alpha 94')).toHaveTextContent(/Grace archived it · /u);
    });
    expect(services.subRooms.subRooms).toHaveBeenCalledWith(PARENT, { archived: true });
    expect(row('Alpha 94')).toHaveTextContent('Work');
    expect(row('Mine')).toHaveTextContent(/You archived it · /u);
    expect(row('Mine')).toHaveTextContent('Uncategorized');
    expect(row('Quiet')).toHaveTextContent(/Archived after 30 quiet days · /u);
    expect(within(row('Alpha 94')).getByRole('link', { name: 'Open Alpha 94' })).toBeVisible();
    expect(within(row('Mine')).queryByRole('link')).not.toBeInTheDocument();
    expect(within(row('Quiet')).queryByRole('button')).not.toBeInTheDocument();

    fireEvent.click(within(row('Mine')).getByRole('button', { name: 'Restore Mine' }));
    await waitFor(() => {
      expect(services.subRooms.restore).toHaveBeenCalledWith({
        parentId: PARENT,
        subRoomId: ARCHIVED.subRooms[1]?.catalogId,
      });
    });
  });
});
