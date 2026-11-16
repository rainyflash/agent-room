// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { BrowserWorkspaceCache } from '@/features/personal-workspace/adapters/browser-workspace-cache';
import { PersonalWorkspaceStore } from '@/features/personal-workspace/application/personal-workspace-store';
import {
  changeWorkspace,
  emptyWorkspace,
  type WorkspaceDocument,
} from '@/features/personal-workspace/domain/workspace-document';
import type { WorkspaceGateway } from '@/features/personal-workspace/domain/workspace-gateway';
import { PersonalWorkspaceProvider } from '@/features/personal-workspace/ui/personal-workspace-provider';
import type { RoomNotice } from '@/features/room-notice/domain/room-notice';
import { RoomNoticeBar, type RoomNoticeBarProps } from '@/features/room-notice/ui/room-notice-bar';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { ok } from '@/shared/result';

const ROOM_ID = '!club:matrix.test';
const SELF = '@mina:matrix.test';
const GRACE = '@grace:matrix.test';
const BRIEF_KEY = `brief:${ROOM_ID}`;
const LEGACY_KEY = `agent-room.room-notice-seen.${ROOM_ID}`;
const CHANGED_AT = Date.parse('2026-11-03T08:00:00.000Z');

const notice = (overrides: Partial<RoomNotice> = {}): RoomNotice => ({
  text: 'Only speak when someone mentions you.\nKeep replies short.\nAsk Grace first.',
  updatedAtUnixMs: CHANGED_AT,
  updatedBy: GRACE,
  ...overrides,
});

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(async () => {
  window.localStorage.clear();
  await i18n.changeLanguage('en');
});

afterEach(cleanup);

function bar(props: Partial<RoomNoticeBarProps> = {}) {
  return (
    <RoomNoticeBar
      memberNames={[{ displayName: 'Grace', matrixUserId: GRACE }]}
      notice={notice()}
      roomId={ROOM_ID}
      selfMatrixUserId={SELF}
      {...props}
    />
  );
}

function renderBar(props: Partial<RoomNoticeBarProps> = {}) {
  return render(<I18nextProvider i18n={i18n}>{bar(props)}</I18nextProvider>);
}

/** 个人工作区：服务器上那一份放在内存里，`hold` 时从服务器读的那一下一直不回来。 */
function workspace(remote: WorkspaceDocument = emptyWorkspace, hold = false) {
  let stored = remote;
  const values = new Map<string, string>();
  const gateway: WorkspaceGateway = {
    scope: () => ({ accountId: SELF, writerId: 'laptop' }),
    read: vi.fn<WorkspaceGateway['read']>(() =>
      hold ? new Promise(() => undefined) : Promise.resolve(ok(stored)),
    ),
    write: vi.fn<WorkspaceGateway['write']>((_scope, document) => {
      stored = document;
      return Promise.resolve(ok(undefined));
    }),
    subscribe: () => () => undefined,
  };
  const store = new PersonalWorkspaceStore(
    gateway,
    new BrowserWorkspaceCache({
      getItem: (key) => values.get(key) ?? null,
      setItem: (key, value) => {
        values.set(key, value);
      },
    }),
  );
  return { store, remote: () => stored };
}

async function renderWithWorkspace(
  store: PersonalWorkspaceStore,
  props: Partial<RoomNoticeBarProps> = {},
) {
  const view = render(
    <I18nextProvider i18n={i18n}>
      <PersonalWorkspaceProvider store={store}>{bar(props)}</PersonalWorkspaceProvider>
    </I18nextProvider>,
  );
  // 等从服务器读完、写回去。
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
  return view;
}

function seenOn(document: WorkspaceDocument, at: number): WorkspaceDocument {
  const changed = changeWorkspace(
    document,
    { kind: 'acknowledged', id: BRIEF_KEY, value: at },
    'phone',
  );
  if (!changed.ok) throw new Error('写不进工作区');
  return changed.value;
}

describe('对话上方的须知', () => {
  it('平时只显示开头两行，没看过的亮点；点开看全文和谁改的，点灭', () => {
    renderBar();
    const region = screen.getByRole('region', { name: 'Room brief' });
    const text = within(region).getByText(/Only speak when someone mentions you/u);
    expect(text).toHaveClass('room-notice__text--clamped');
    expect(within(region).getByText('Changed since you last looked')).toBeInTheDocument();
    expect(within(region).queryByText(/Changed by/u)).toBeNull();

    fireEvent.click(within(region).getByRole('button', { name: /Show the whole brief/u }));

    expect(text).not.toHaveClass('room-notice__text--clamped');
    expect(within(region).getByText(/^Changed by Grace · /u)).toBeVisible();
    expect(within(region).queryByText('Changed since you last looked')).toBeNull();
    expect(within(region).getByRole('button', { name: /Show less/u })).toHaveAttribute(
      'aria-expanded',
      'true',
    );

    // 没有个人工作区的页面记在这个浏览器里。
    cleanup();
    renderBar();
    expect(screen.queryByText('Changed since you last looked')).toBeNull();
  });

  it('改了新的一版又亮；自己改的不亮，写“你”', () => {
    window.localStorage.setItem(LEGACY_KEY, String(CHANGED_AT));
    renderBar({ notice: notice({ updatedAtUnixMs: Date.parse('2026-11-03T09:00:00.000Z') }) });
    expect(screen.getByText('Changed since you last looked')).toBeInTheDocument();

    cleanup();
    renderBar({
      notice: notice({ updatedAtUnixMs: Date.parse('2026-11-03T10:00:00.000Z'), updatedBy: SELF }),
    });
    expect(screen.queryByText('Changed since you last looked')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: /Show the whole brief/u }));
    expect(screen.getByText(/^Changed by you · /u)).toBeVisible();
  });

  it('认不出改的人时写“房主或管理员”', () => {
    renderBar({ memberNames: undefined });
    fireEvent.click(screen.getByRole('button', { name: /Show the whole brief/u }));
    expect(screen.getByText(/^Changed by the owner or an admin · /u)).toBeVisible();
  });

  it('房主和管理员才有“编辑”', () => {
    renderBar();
    expect(screen.queryByRole('button', { name: 'Edit brief' })).toBeNull();

    cleanup();
    const onEdit = vi.fn();
    renderBar({ onEdit });
    fireEvent.click(screen.getByRole('button', { name: 'Edit brief' }));
    expect(onEdit).toHaveBeenCalledOnce();
  });
});

describe('看没看过记在个人工作区里', () => {
  it('点开以后记进工作区，不记在这个浏览器里；换一台设备打开不再亮', async () => {
    const laptop = workspace();
    await renderWithWorkspace(laptop.store);
    expect(screen.getByText('Changed since you last looked')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /Show the whole brief/u }));
    await act(async () => {
      await Promise.resolve();
    });

    expect(screen.queryByText('Changed since you last looked')).toBeNull();
    expect(laptop.store.getSnapshot().index.acknowledgedAt.get(BRIEF_KEY)).toBe(CHANGED_AT);
    expect(window.localStorage.getItem(LEGACY_KEY)).toBeNull();

    cleanup();
    const phone = workspace(laptop.remote());
    await renderWithWorkspace(phone.store);
    expect(screen.getByRole('region', { name: 'Room brief' })).toBeVisible();
    expect(screen.queryByText('Changed since you last looked')).toBeNull();
  });

  it('别的设备上看过的是旧的一版：照样亮', async () => {
    const phone = workspace(seenOn(emptyWorkspace, CHANGED_AT - 60_000));
    await renderWithWorkspace(phone.store);
    expect(screen.getByText('Changed since you last looked')).toBeInTheDocument();
  });

  it('这个浏览器以前记着的看过搬进工作区，搬完删掉', async () => {
    window.localStorage.setItem(LEGACY_KEY, String(CHANGED_AT));
    const laptop = workspace();
    await renderWithWorkspace(laptop.store);

    expect(screen.queryByText('Changed since you last looked')).toBeNull();
    expect(laptop.store.getSnapshot().index.acknowledgedAt.get(BRIEF_KEY)).toBe(CHANGED_AT);
    expect(window.localStorage.getItem(LEGACY_KEY)).toBeNull();
  });

  it('搬的时候工作区里已经是更新的一版：不往回改', async () => {
    window.localStorage.setItem(LEGACY_KEY, String(CHANGED_AT - 60_000));
    const laptop = workspace(seenOn(emptyWorkspace, CHANGED_AT));
    await renderWithWorkspace(laptop.store);

    expect(screen.queryByText('Changed since you last looked')).toBeNull();
    expect(laptop.store.getSnapshot().index.acknowledgedAt.get(BRIEF_KEY)).toBe(CHANGED_AT);
  });

  it('工作区还在从服务器读：先不亮，免得亮一下又灭', async () => {
    const slow = workspace(emptyWorkspace, true);
    await renderWithWorkspace(slow.store);
    expect(screen.getByRole('region', { name: 'Room brief' })).toBeVisible();
    expect(screen.queryByText('Changed since you last looked')).toBeNull();
  });
});
