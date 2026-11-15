// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { RoomUnread } from '@/features/inbox/domain/inbox';
import type { SubRoom, SubRoomList } from '@/features/private-rooms/domain/sub-rooms';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

import { SubRoomPlaces, type SubRoomPlacesProps } from './sub-room-places';

const unread = new Map<string, { roomId: string; unread: RoomUnread }>();

vi.mock('@/features/inbox/ui/room-unread-mark', () => ({
  useCatalogUnread: (catalogId: string) => unread.get(catalogId),
}));

vi.mock('@tanstack/react-router', () => ({
  Link: ({
    children,
    params,
    to,
    ...rest
  }: {
    readonly children: ReactNode;
    readonly params: { readonly catalogId: string; readonly roomId?: string };
    readonly to: string;
  }) => (
    <a
      href={to.replace('$catalogId', params.catalogId).replace('$roomId', params.roomId ?? '')}
      {...rest}
    >
      {children}
    </a>
  ),
}));

const PARENT = '0198b601-77a1-7bb8-83eb-a8fe68c97e40';
const WORK = '0198b601-77a1-7bb8-83eb-a8fe68c97e60';
const OPS = '0198b601-77a1-7bb8-83eb-a8fe68c97e61';
const PERSON = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';

function subRoom(id: string, name: string, extra: Partial<SubRoom> = {}): SubRoom {
  return {
    catalogId: `0198b601-77a1-7bb8-83eb-a8fe68c97e${id}`,
    name,
    description: '',
    access: 'open',
    hidden: false,
    createdByPrincipalId: PERSON,
    createdAtUnixMs: 1,
    managerPrincipalIds: [],
    joined: true,
    banned: false,
    ...extra,
  };
}

const RELEASE = subRoom('51', 'Release', { categoryId: WORK });
const BACKUPS = subRoom('52', 'Backups', { access: 'request', categoryId: OPS });
const LOOSE = subRoom('53', 'Loose ends');
const ELSEWHERE = subRoom('54', 'Not mine', { joined: false, categoryId: WORK });

const LIST: SubRoomList = {
  parent: { catalogId: PARENT, name: 'Agent Room' },
  canCreate: true,
  categories: [
    { categoryId: OPS, name: 'Ops', position: 1 },
    { categoryId: WORK, name: 'Work', position: 0 },
  ],
  subRooms: [RELEASE, BACKUPS, LOOSE, ELSEWHERE],
};

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(() => {
  cleanup();
  unread.clear();
  window.localStorage.clear();
});

function renderPlaces(overrides: Partial<SubRoomPlacesProps> = {}) {
  const props: SubRoomPlacesProps = {
    browse: null,
    currentCatalogId: PARENT,
    currentSelected: true,
    currentUnread: 0,
    list: LIST,
    onCreate: null,
    onNavigate: vi.fn(),
    onSelectCurrent: vi.fn(),
    ...overrides,
  };
  render(
    <I18nextProvider i18n={i18n}>
      <SubRoomPlaces {...props} />
    </I18nextProvider>,
  );
  return props;
}

describe('左栏里的子房间', () => {
  it('只放你在的，未分类的在最上面，分类按先后，没你在的分类不出现', () => {
    renderPlaces();

    const nav = screen.getByRole('navigation', { name: 'Sub-rooms' });
    const names = within(nav)
      .getAllByRole('link')
      .map((link) => link.getAttribute('aria-label'));
    expect(names).toEqual(['Loose ends', 'Release', 'Backups']);
    expect(
      within(nav)
        .getAllByRole('button')
        .map((button) => button.textContent),
    ).toEqual(['Work', 'Ops']);
    expect(screen.queryByText('Not mine')).not.toBeInTheDocument();
  });

  it('有门的读得出来；收件箱知道它的 Matrix 房间就直接打开', () => {
    unread.set(BACKUPS.catalogId, {
      roomId: '!backups:matrix.test',
      unread: { catalogId: BACKUPS.catalogId, count: 0, more: false, mentioned: false },
    });
    renderPlaces();

    const backups = screen.getByRole('link', { name: 'Backups' });
    expect(backups).toHaveAccessibleDescription('Has a door');
    expect(backups).toHaveAttribute(
      'href',
      `/lobby/${BACKUPS.catalogId}/instance/!backups:matrix.test`,
    );
    expect(screen.getByRole('link', { name: 'Release' })).toHaveAttribute(
      'href',
      `/lobby/${RELEASE.catalogId}`,
    );
  });

  it('没读的写灰色的数，提到你的是红底的', () => {
    unread.set(RELEASE.catalogId, {
      roomId: '!release:matrix.test',
      unread: { catalogId: RELEASE.catalogId, count: 7, more: false, mentioned: false },
    });
    unread.set(LOOSE.catalogId, {
      roomId: '!loose:matrix.test',
      unread: { catalogId: LOOSE.catalogId, count: 2, more: false, mentioned: true },
    });
    renderPlaces();

    const release = screen.getByRole('link', { name: 'Release' });
    expect(release).toHaveAccessibleDescription('7 unread');
    expect(release).toHaveAttribute('data-unread', 'true');
    expect(release.querySelector('.sub-room-places__count')).toHaveTextContent('7');
    const loose = screen.getByRole('link', { name: 'Loose ends' });
    expect(loose).toHaveAccessibleDescription('Mentions you, 2 unread');
    expect(loose.querySelector('.sub-room-places__mention')).toBeInTheDocument();
  });

  it('眼前这间子房间：在它的对话里时是“你在这儿”、不标没读，点它回到它的对话', () => {
    unread.set(RELEASE.catalogId, {
      roomId: '!release:matrix.test',
      unread: { catalogId: RELEASE.catalogId, count: 4, more: false, mentioned: true },
    });
    const props = renderPlaces({ currentCatalogId: RELEASE.catalogId });

    const release = screen.getByRole('button', { name: 'Release' });
    expect(release).toHaveAttribute('aria-current', 'page');
    expect(release).not.toHaveAttribute('data-unread');
    fireEvent.click(release);
    expect(props.onNavigate).toHaveBeenCalledOnce();
    expect(props.onSelectCurrent).toHaveBeenCalledOnce();
  });

  it('在私聊里时，眼前这间照这一页数的标没读', () => {
    renderPlaces({ currentCatalogId: RELEASE.catalogId, currentSelected: false, currentUnread: 3 });

    const release = screen.getByRole('button', { name: 'Release' });
    expect(release).not.toHaveAttribute('aria-current');
    expect(release).toHaveAccessibleDescription('3 unread');
  });

  it('眼前这间你还没进（有门的卡片）也列出来', () => {
    renderPlaces({ currentCatalogId: ELSEWHERE.catalogId });

    expect(screen.getByRole('button', { name: 'Not mine' })).toBeVisible();
  });

  it('分类能收起，这台设备记着；收起来时眼前这一间照样留着', () => {
    renderPlaces({ currentCatalogId: RELEASE.catalogId });

    fireEvent.click(screen.getByRole('button', { name: 'Work' }));
    expect(screen.getByRole('button', { name: 'Work' })).toHaveAttribute('aria-expanded', 'false');
    expect(screen.getByRole('button', { name: 'Release' })).toBeVisible();

    fireEvent.click(screen.getByRole('button', { name: 'Ops' }));
    expect(screen.queryByRole('link', { name: 'Backups' })).not.toBeInTheDocument();

    cleanup();
    renderPlaces();
    expect(screen.getByRole('button', { name: 'Ops' })).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByRole('link', { name: 'Backups' })).not.toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'Release' })).not.toBeInTheDocument();
  });

  it('最上面“浏览子房间”：在父房间里点了打开那一页，带着几间、没见过的新的', () => {
    const onOpen = vi.fn();
    renderPlaces({
      browse: { count: 4, onOpen, parentMatrixRoomId: undefined, selected: true, unseen: 2 },
    });

    const browse = screen.getByRole('button', { name: 'Browse sub-rooms (4)' });
    expect(browse).toHaveAttribute('aria-current', 'page');
    expect(browse).toHaveAccessibleDescription('2 new you haven’t seen');
    fireEvent.click(browse);
    expect(onOpen).toHaveBeenCalledOnce();
  });

  it('在子房间里“浏览子房间”去父房间的那一页', () => {
    renderPlaces({
      browse: {
        count: 4,
        onOpen: null,
        parentMatrixRoomId: '!parent:matrix.test',
        selected: false,
        unseen: 0,
      },
      currentCatalogId: RELEASE.catalogId,
    });

    const browse = screen.getByRole('link', { name: 'Browse sub-rooms (4)' });
    expect(browse).toHaveAttribute('href', `/lobby/${PARENT}/instance/!parent:matrix.test`);
    expect(browse).not.toHaveAccessibleDescription();
  });

  it('最下面“新建子房间”：建得了时有，点了先收起下拉再打开对话框', () => {
    const onCreate = vi.fn();
    const onNavigate = vi.fn();
    renderPlaces({ onCreate, onNavigate });

    const create = screen.getByRole('button', { name: 'New sub-room' });
    // 排在你在的子房间后面，是最后一行。
    expect([...screen.getByRole('navigation').querySelectorAll('a, button')].at(-1)).toBe(create);
    fireEvent.click(create);
    expect(onNavigate).toHaveBeenCalledOnce();
    expect(onCreate).toHaveBeenCalledOnce();
  });

  it('一间都不在、也建不了时什么都不画；建得了时留着“新建子房间”', () => {
    renderPlaces({ list: { ...LIST, subRooms: [ELSEWHERE] }, onCreate: vi.fn() });
    expect(screen.getByRole('button', { name: 'New sub-room' })).toBeVisible();
    cleanup();

    renderPlaces({ list: { ...LIST, subRooms: [ELSEWHERE] } });
    expect(screen.queryByRole('navigation')).not.toBeInTheDocument();
  });

  it('你一间都不在时什么都不画', () => {
    renderPlaces({
      list: { ...LIST, subRooms: [ELSEWHERE] },
    });

    expect(screen.queryByRole('navigation')).not.toBeInTheDocument();
  });
});
