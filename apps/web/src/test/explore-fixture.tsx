import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
  useLocation,
  useNavigate,
} from '@tanstack/react-router';
import { useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { I18nextProvider } from 'react-i18next';

import '@agent-room/ui-system/styles.css';
import '@/app/styles.css';

import {
  defaultExploreFilter,
  type ExploreFilter,
  type ExploreRoom,
  type ExploreRoomDetail,
} from '@/features/explore/domain/explore';
import type { ExploreRoomDetailState } from '@/features/explore/ui/explore-room-panel';
import {
  ExploreView,
  type ExploreAccount,
  type ExploreRoomsState,
} from '@/features/explore/ui/explore-view';
import { useApprovedJoin } from '@/features/explore/ui/use-approved-join';
import type { ExploreActionKind, ExploreActions } from '@/features/explore/ui/use-explore-actions';
import type { PublicRoomSummary } from '@/features/room-directory/domain/public-room-directory';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { ok } from '@/shared/result';

/**
 * 发现页的夹具（specs/public-rooms/design.md“发现页”）：
 * - 默认：登录了，几间公开房间，你在不同的处境里（已加入、在等批准、收到邀请、被封禁、只能邀请），还有更多；
 * - `?signed-out`：没登录，顶栏是“登录”“注册”，卡片上是“登录后加入”（记下回来接着加入的那间房）；
 * - `?empty`：什么都没搜到；`?failed`：没读到；`?limited`：被限流；`?loading`：还在读。
 * 点卡片打开详情（`/explore/<分享码>`），加入、申请、撤回照着做（见 `useFixtureActions`）。
 * `window.__agentRoomFixtureControls.approveRequests()` 像房主批准了一样，把这一页上申请的都改成收到邀请。
 * 点筛选、排序、搜索在这一页里照着筛；点了什么记在 `document.body.dataset` 上。语言跟着浏览器走。
 */
const search = new URLSearchParams(location.search);
const now = Date.now();
const minute = 60_000;

const lobbies: readonly PublicRoomSummary[] = [
  {
    activeInstanceCount: 1,
    catalogId: '0198b601-77a2-7f41-b4f4-940f291951b8',
    description: 'The default public lobby.',
    language: 'en',
    name: 'Default public lobby',
    onlineAgentCount: 12,
    slug: 'default-public',
  },
  {
    activeInstanceCount: 1,
    catalogId: '0198b601-77a3-74f1-b4f4-940f291951b9',
    description: '中文公共大厅。',
    language: 'zh-CN',
    name: '中文公共大厅',
    onlineAgentCount: 1,
    slug: 'zh-public',
  },
];

function room(
  index: number,
  name: string,
  extra: Partial<ExploreRoom> & Pick<ExploreRoom, 'joinMode'>,
): ExploreRoom {
  const suffix = index.toString(16).padStart(12, '0');
  const code = `Rm${index.toString().padStart(4, '0')}`;
  return {
    agentCount: 2,
    catalogId: `0198c000-0000-7000-8000-${suffix}`,
    createdAtUnixMs: now - index * 24 * 60 * minute,
    description: '',
    endToEndEncrypted: false,
    lastMessageAtUnixMs: now - index * 7 * minute,
    memberCount: 3,
    name,
    ownerName: 'Grace',
    shareCode: code,
    shareUrl: `https://agentroom.chat/r/${code}`,
    viewer: null,
    ...extra,
  };
}

const allRooms: readonly ExploreRoom[] = [
  room(1, 'Rust help desk', {
    agentCount: 4,
    description:
      'Ask about borrow checker errors, async runtimes and unsafe code. Agents here read compiler output and suggest the smallest fix first; people review before anything is merged. Long questions are welcome.',
    joinMode: 'open',
    memberCount: 18,
  }),
  room(2, '周末读书会', {
    description: '每周六晚上一起读一章，Agent 帮忙整理笔记。',
    joinMode: 'request',
    memberCount: 9,
    ownerName: '林',
    viewer: 'pending',
  }),
  room(3, 'Design critiques', {
    agentCount: 1,
    description: 'Post a screen, get two people and one agent to look at it.',
    joinMode: 'request',
    memberCount: 6,
  }),
  room(4, 'Agent tinkering', {
    description: 'Build small agents and show them off.',
    endToEndEncrypted: true,
    joinMode: 'open',
    lastMessageAtUnixMs: null,
    memberCount: 2,
    viewer: 'joined',
  }),
  room(5, 'Board game nights', {
    description: 'Invite only: we keep the table small.',
    joinMode: 'invite_only',
    memberCount: 5,
  }),
  room(6, 'Paper club', {
    description: 'You were invited by the owner.',
    joinMode: 'invite_only',
    viewer: 'invited',
  }),
  room(7, 'Old arguments', { joinMode: 'open', viewer: 'banned' }),
  room(9, 'Changing rooms', { description: 'Just switched to asking first.', joinMode: 'open' }),
];

function visibleRooms(filter: ExploreFilter): readonly ExploreRoom[] {
  const term = filter.q.trim().toLowerCase();
  const matching = allRooms.filter((candidate) => {
    const text = `${candidate.name} ${candidate.description} ${candidate.ownerName}`.toLowerCase();
    if (term !== '' && !text.includes(term)) return false;
    switch (filter.quick) {
      case 'open':
      case 'request':
        return candidate.joinMode === filter.quick;
      case 'active':
        return (
          candidate.lastMessageAtUnixMs !== null &&
          now - candidate.lastMessageAtUnixMs < 60 * minute
        );
      case 'plain':
        return !candidate.endToEndEncrypted;
      case 'all':
        return true;
    }
  });
  return filter.sort === 'members'
    ? [...matching].sort((left, right) => right.memberCount - left.memberCount)
    : filter.sort === 'newest'
      ? [...matching].sort((left, right) => right.createdAtUnixMs - left.createdAtUnixMs)
      : matching;
}

const detailFacts: Pick<ExploreRoomDetail, 'admins' | 'online'> = {
  admins: ['Lin', 'Mo'],
  online: { atLeast: false, count: 3 },
};
const lateRoom = room(8, 'Late arrivals', { description: 'Page two.', joinMode: 'open' });

function fixtureAccount(): ExploreAccount {
  return search.has('signed-out')
    ? {
        kind: 'signed-out',
        onRegister: () => {
          document.body.dataset.fixtureAuthentication = 'register';
        },
        onSignIn: () => {
          document.body.dataset.fixtureAuthentication = 'sign-in';
        },
        onSignInToJoin: (shareCode) => {
          document.body.dataset.fixtureAuthentication = `sign-in:/explore/${shareCode}?join=1`;
        },
        registrationOpen: true,
      }
    : { kind: 'signed-in' };
}

function fixtureRooms(rooms: readonly ExploreRoom[], extra: boolean): ExploreRoomsState {
  if (search.has('loading')) return { kind: 'loading' };
  if (search.has('failed')) {
    return { kind: 'failed', failure: { code: 'public_room.unreachable', retryable: true } };
  }
  if (search.has('limited')) {
    return {
      kind: 'failed',
      failure: { code: 'public_rooms.rate_limited', retryable: true, retryAfterSeconds: 30 },
    };
  }
  if (search.has('empty')) return { kind: 'ready', more: 'none', rooms: [] };
  return { kind: 'ready', more: extra ? 'none' : 'available', rooms };
}

/**
 * 加入、申请、撤回在这一页里照着做：直接进的跳到房间；“Changing rooms”加入时说现在要申请；申请以后
 * 变成在等批准，留的话记在 `document.body.dataset.fixtureRequest`。
 */
function useFixtureActions(): {
  readonly actions: ExploreActions;
  readonly viewers: Readonly<Record<string, ExploreRoom['viewer']>>;
} {
  const navigate = useNavigate();
  const [viewers, setViewers] = useState<Record<string, ExploreRoom['viewer']>>({});
  const [busy, setBusy] = useState<ExploreActions['busy']>(null);
  const [failure, setFailure] = useState<ExploreActions['failure']>(null);
  useEffect(() => {
    Object.defineProperty(window, '__agentRoomFixtureControls', {
      configurable: true,
      value: {
        approveRequests: () => {
          setViewers((current) =>
            Object.fromEntries(
              Object.entries(current).map(([catalogId, viewer]) => [
                catalogId,
                viewer === 'pending' ? 'invited' : viewer,
              ]),
            ),
          );
        },
      },
    });
  }, []);
  const later = (catalogId: string, kind: ExploreActionKind, then: () => void) => {
    setBusy({ catalogId, kind });
    setFailure(null);
    window.setTimeout(() => {
      setBusy(null);
      then();
    }, 200);
  };
  return {
    actions: {
      busy,
      failure,
      onJoin: (target) => {
        later(target.catalogId, 'join', () => {
          if (target.name === 'Changing rooms') {
            setFailure({ catalogId: target.catalogId, code: 'room_join.approval_required' });
            return;
          }
          void navigate({ params: { catalogId: target.catalogId }, to: '/lobby/$catalogId' });
        });
      },
      onRequest: (target, message) => {
        later(target.catalogId, 'request', () => {
          document.body.dataset.fixtureRequest = message;
          setViewers((current) => ({ ...current, [target.catalogId]: 'pending' }));
        });
      },
      onWithdraw: (target) => {
        later(target.catalogId, 'withdraw', () => {
          setViewers((current) => ({ ...current, [target.catalogId]: null }));
        });
      },
    },
    viewers,
  };
}

function Fixture() {
  const [filter, setFilter] = useState<ExploreFilter>(defaultExploreFilter);
  const [extra, setExtra] = useState(false);
  const navigate = useNavigate();
  const opened = useLocation({ select: (location) => location.pathname });
  document.body.dataset.fixtureOpened = opened;
  const account = fixtureAccount();
  const { actions, viewers } = useFixtureActions();
  const withViewer = (candidate: ExploreRoom): ExploreRoom =>
    candidate.catalogId in viewers
      ? { ...candidate, viewer: viewers[candidate.catalogId] ?? null }
      : candidate;
  const listed = [...visibleRooms(filter), ...(extra ? [lateRoom] : [])].map(withViewer);
  const code = /^\/explore\/([^/]+)$/u.exec(opened)?.[1] ?? null;
  const selected = [...allRooms, lateRoom].find((candidate) => candidate.shareCode === code);
  const detail: ExploreRoomDetailState =
    selected === undefined
      ? { kind: 'failed', failure: { code: 'public_room.not_found', retryable: false } }
      : { kind: 'ready', room: { ...withViewer(selected), ...detailFacts } };
  // 像发现页那样，批准了的直接进房间。
  useApprovedJoin({
    detail: detail.kind === 'ready' ? ok(detail.room) : undefined,
    onJoin: account.kind === 'signed-in' ? actions.onJoin : undefined,
    pages: [ok({ nextCursor: null, rooms: listed })],
  });
  return (
    <ExploreView
      account={account}
      actions={account.kind === 'signed-in' ? actions : undefined}
      filter={filter}
      lobbies={lobbies}
      now={now}
      onCreatePublicRoom={
        account.kind === 'signed-in'
          ? () => {
              document.body.dataset.fixtureCreate = 'public';
            }
          : undefined
      }
      onFilterChange={(next) => {
        document.body.dataset.fixtureFilter = JSON.stringify(next);
        setFilter(next);
      }}
      onLoadMore={() => {
        setExtra(true);
      }}
      onRetry={() => {
        document.body.dataset.fixtureRetry = 'true';
      }}
      panel={
        code === null
          ? undefined
          : {
              detail,
              onClose: () => {
                void navigate({ to: '/explore' });
              },
            }
      }
      rooms={fixtureRooms(listed, extra)}
    />
  );
}

// 发现页一直在根上，换地址（打开详情、进房间）时它的状态还在。
const rootRoute = createRootRoute({ component: Fixture });
const placeholder = (path: string) =>
  createRoute({ component: () => null, getParentRoute: () => rootRoute, path });
const routeTree = rootRoute.addChildren(
  [
    '/explore',
    '/explore/$code',
    '/',
    '/rooms',
    '/inbox',
    '/workspace',
    '/settings/$section',
    '/lobby/$catalogId',
    '/watch',
    '/watch/$slug',
  ].map(placeholder),
);
const router = createRouter({
  // `?open=<分享码>` 一打开就是那间房的详情，像别人发来的链接。
  history: createMemoryHistory({
    initialEntries: [search.has('open') ? `/explore/${search.get('open') ?? ''}` : '/explore'],
  }),
  routeTree,
});

async function bootstrapFixture(): Promise<void> {
  // 语言跟着浏览器走：验收里用 `locale: 'zh-CN'` 看中文。
  await initializeI18n(window.localStorage);
  const root = document.querySelector('#root');
  if (!(root instanceof HTMLElement)) {
    throw new Error('发现页测试根节点不存在。');
  }
  createRoot(root).render(
    <I18nextProvider i18n={i18n}>
      <RouterProvider router={router} />
    </I18nextProvider>,
  );
}

void bootstrapFixture();
