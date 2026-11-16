// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { conversationFixture } from '@/features/conversation/testing/conversation-fixture';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { BrowserWorkspaceCache } from '@/features/personal-workspace/adapters/browser-workspace-cache';
import { PersonalWorkspaceStore } from '@/features/personal-workspace/application/personal-workspace-store';
import {
  changeWorkspace,
  emptyWorkspace,
  type WorkspaceDocument,
} from '@/features/personal-workspace/domain/workspace-document';
import type { WorkspaceGateway } from '@/features/personal-workspace/domain/workspace-gateway';
import { PersonalWorkspaceProvider } from '@/features/personal-workspace/ui/personal-workspace-provider';
import type { RoomPins } from '@/features/room-pins/domain/room-pins';
import { RoomPinsBar, type RoomPinsBarProps } from '@/features/room-pins/ui/room-pins-bar';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { ok } from '@/shared/result';

// 这间房这台设备上加载了的消息：测试里直接给。
let loaded: readonly RoomMessageSignal[] = [];
vi.mock('@/features/messages/ui/room-messages-context', () => ({
  useRoomMessages: () => ({ state: { kind: 'ready', room: { messages: loaded } }, store: {} }),
}));

const ROOM_ID = '!club:matrix.test';
const SELF = '@mina:matrix.test';
const GRACE = '@grace:matrix.test';
const PINS_KEY = `pins:${ROOM_ID}`;
const CHANGED_AT = Date.parse('2026-11-16T08:00:00.000Z');

const said = (id: string, text: string, options: Partial<RoomMessageSignal> = {}) =>
  conversationFixture(id, {
    roomId: ROOM_ID,
    actor: {
      kind: 'human',
      principalId: '0198b601-77a1-7bb8-83eb-a8fe68c97e46',
      displayName: 'Grace',
      matrixUserId: GRACE,
      provenance: 'human',
    },
    preview: {
      title: 'Conversation',
      summary: text,
      conversation: { text, mentions: [] },
      contentType: 'text/plain',
      riskFlags: [],
      sensitivity: 'normal',
    },
    ...options,
  });

const pins = (overrides: Partial<RoomPins> = {}): RoomPins => ({
  entries: [
    { eventId: '$recent', pinnedBy: GRACE, pinnedAtUnixMs: CHANGED_AT },
    { eventId: '$older', pinnedBy: GRACE, pinnedAtUnixMs: CHANGED_AT - 60_000 },
  ],
  updatedAtUnixMs: CHANGED_AT,
  updatedBy: GRACE,
  ...overrides,
});

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(async () => {
  window.localStorage.clear();
  loaded = [said('recent', 'Ship the release on Friday.')];
  await i18n.changeLanguage('en');
});

afterEach(cleanup);

function nth(list: readonly HTMLElement[], index: number): HTMLElement {
  const element = list[index];
  if (element === undefined) throw new Error('列表里没有这一条');
  return element;
}

function bar(props: Partial<RoomPinsBarProps> = {}) {
  return (
    <RoomPinsBar
      onJump={() => undefined}
      pins={pins()}
      roomId={ROOM_ID}
      selfMatrixUserId={SELF}
      {...props}
    />
  );
}

/** 个人工作区：服务器上那一份放在内存里。 */
function workspace(remote: WorkspaceDocument = emptyWorkspace) {
  let stored = remote;
  const values = new Map<string, string>();
  const gateway: WorkspaceGateway = {
    scope: () => ({ accountId: SELF, writerId: 'laptop' }),
    read: vi.fn<WorkspaceGateway['read']>(() => Promise.resolve(ok(stored))),
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
  props: Partial<RoomPinsBarProps> = {},
) {
  const view = render(
    <I18nextProvider i18n={i18n}>
      <PersonalWorkspaceProvider store={store}>{bar(props)}</PersonalWorkspaceProvider>
    </I18nextProvider>,
  );
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
  return view;
}

describe('须知下面置顶的那一行', () => {
  it('平时一行：几条、最新那一句是谁说的和开头，没看过的亮点；点开列出每一句、点灭', () => {
    const onJump = vi.fn<(eventId: string) => void>();
    render(<I18nextProvider i18n={i18n}>{bar({ onJump })}</I18nextProvider>);
    const region = screen.getByRole('region', { name: 'Pinned messages' });
    const toggle = within(region).getByRole('button', { name: /Show pinned messages/u });
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    expect(within(region).getByText('2')).toBeInTheDocument();
    expect(within(region).getByText('Grace: Ship the release on Friday.')).toBeInTheDocument();
    expect(within(region).getByText('Changed since you last looked')).toBeInTheDocument();
    expect(within(region).getByRole('list', { hidden: true })).not.toBeVisible();

    fireEvent.click(toggle);

    expect(within(region).getByRole('button', { name: /Hide pinned messages/u })).toHaveAttribute(
      'aria-expanded',
      'true',
    );
    expect(within(region).queryByText('Changed since you last looked')).toBeNull();
    // 点开以后头一行不再写开头，列表里写全。
    expect(within(region).queryByText('Grace: Ship the release on Friday.')).toBeNull();
    const items = within(region).getAllByRole('listitem');
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent('Grace');
    expect(items[0]).toHaveTextContent('Ship the release on Friday.');
    expect(items[1]).toHaveTextContent('Not loaded yet');

    fireEvent.click(within(nth(items, 0)).getByRole('button'));
    fireEvent.click(within(nth(items, 1)).getByRole('button'));
    expect(onJump.mock.calls).toEqual([['$recent'], ['$older']]);
  });

  it('删了的写“这句已经删了”、点不了；资料卡片不在对话里，当没加载', () => {
    loaded = [
      said('recent', 'Ship the release on Friday.', { lifecycle: 'redacted' }),
      said('older', 'A file', {
        preview: {
          title: 'Report',
          summary: 'Weekly report',
          contentType: 'text/markdown',
          riskFlags: [],
          sensitivity: 'normal',
        },
      }),
    ];
    render(<I18nextProvider i18n={i18n}>{bar()}</I18nextProvider>);
    const region = screen.getByRole('region', { name: 'Pinned messages' });
    // 最新那一句删了：头一行不写开头。
    expect(region.querySelector('.room-pins__summary')).toBeNull();
    fireEvent.click(within(region).getByRole('button', { name: /Show pinned messages/u }));
    const items = within(region).getAllByRole('listitem');
    expect(items[0]).toHaveTextContent('This message was deleted');
    expect(within(nth(items, 0)).queryByRole('button')).toBeNull();
    expect(items[1]).toHaveTextContent('Not loaded yet');
  });

  it('自己改的不亮点', () => {
    render(
      <I18nextProvider i18n={i18n}>{bar({ pins: pins({ updatedBy: SELF }) })}</I18nextProvider>,
    );
    expect(screen.queryByText('Changed since you last looked')).toBeNull();
  });

  it('看过哪一次记在个人工作区的 pins:<房间>；别的设备上看过这一次的不亮，改了以后又亮', async () => {
    const fresh = workspace();
    await renderWithWorkspace(fresh.store);
    const region = screen.getByRole('region', { name: 'Pinned messages' });
    expect(within(region).getByText('Changed since you last looked')).toBeInTheDocument();
    fireEvent.click(within(region).getByRole('button', { name: /Show pinned messages/u }));
    await act(async () => {
      await Promise.resolve();
    });
    expect(fresh.store.getSnapshot().index.acknowledgedAt.get(PINS_KEY)).toBe(CHANGED_AT);
    cleanup();

    const seen = changeWorkspace(
      emptyWorkspace,
      { kind: 'acknowledged', id: PINS_KEY, value: CHANGED_AT },
      'phone',
    );
    if (!seen.ok) throw new Error('写不进工作区');
    const elsewhere = workspace(seen.value);
    const view = await renderWithWorkspace(elsewhere.store);
    expect(screen.queryByText('Changed since you last looked')).toBeNull();

    view.rerender(
      <I18nextProvider i18n={i18n}>
        <PersonalWorkspaceProvider store={elsewhere.store}>
          {bar({ pins: pins({ updatedAtUnixMs: CHANGED_AT + 1 }) })}
        </PersonalWorkspaceProvider>
      </I18nextProvider>,
    );
    expect(screen.getByText('Changed since you last looked')).toBeInTheDocument();
  });
});
