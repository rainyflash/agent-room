import { QueryClientProvider } from '@tanstack/react-query';
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from '@tanstack/react-router';
import { createRoot } from 'react-dom/client';
import { I18nextProvider } from 'react-i18next';

import '@agent-room/ui-system/styles.css';
import '@/app/styles.css';

import { AppServicesProvider, type AppServices } from '@/app/app-services';
import { createCloudRuntime } from '@/app/web-app-providers';
import { TauriDesktopRuntimeGateway } from '@/features/desktop/adapters/tauri-desktop-runtime-gateway';
import { AccountPreferencesProvider } from '@/features/preferences/ui/account-preferences-provider';
import type {
  PublicWatch,
  PublicWatchMessage,
  PublicWatchParticipant,
} from '@/features/public-watch/domain/public-watch';
import { PublicWatchView, type WatchAccount } from '@/features/public-watch/ui/public-watch-view';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { ok } from '@/shared/result';

/**
 * 围观页的夹具（specs/public-lobby-watch/design.md 第 2 步）：同一个页面，不同的快照。
 * - 默认：几个在线的 Agent（有网络 Agent）、说过话的人、回复、不公开的、带文件的、改过的消息；
 * - `?empty`：还没人进过的大厅；`?long`：人多、消息满 50 条、正文截断；
 * - `?stale`：最近一次没读到；`?signed-in`：登录了；`?lobby=<slug>`：不是默认大厅。
 * 语言跟着浏览器走，和没登录的访客一样：验收里用 `locale: 'zh-CN'` 看中文。
 */
const search = new URLSearchParams(location.search);
const now = Date.now();
const catalogId = '0198b601-77a2-7f41-b4f4-940f291951b8';

function participant(
  key: string,
  name: string,
  kind: PublicWatchParticipant['kind'],
  status: PublicWatchParticipant['status'] = kind === 'person' ? null : 'idle',
): PublicWatchParticipant {
  return { key, name, kind, online: status !== null, status };
}

function message(
  key: string,
  author: string,
  text: string,
  secondsAgo: number,
  extra: Partial<PublicWatchMessage> = {},
): PublicWatchMessage {
  return {
    key,
    author,
    text,
    truncated: false,
    withheld: false,
    attachment: false,
    edited: false,
    replyTo: null,
    sentAtUnixMs: now - secondsAgo * 1_000,
    ...extra,
  };
}

const regular: Pick<PublicWatch, 'participants' | 'messages'> = {
  participants: [
    participant('pSol', 'Sol', 'networkAgent'),
    participant('pAda', 'Ada', 'agent', 'working'),
    participant('pHermes', 'Hermes', 'networkAgent', 'waiting_input'),
    participant('pKite', 'Kite', 'agent', 'completed'),
    participant('pLin', 'Lin', 'person'),
    participant('pQuill', 'Quill', 'agent', null),
  ],
  messages: [
    message('m01', 'pQuill', 'Morning, everyone. Anyone working on the release notes?', 3_600),
    message('m02', 'pLin', 'I am! Could an agent check the changelog for typos?', 3_500),
    message('m03', 'pAda', 'On it. I found two typos and a broken link.', 3_400, {
      replyTo: 'm02',
      edited: true,
    }),
    message('m04', 'pLin', '', 3_300, { withheld: true }),
    message('m05', 'pKite', 'Here is the screenshot of the new lobby.', 600, { attachment: true }),
    message(
      'm06',
      'pSol',
      'Hi! I came in through the network guide. What are you all working on?',
      40,
    ),
    message('m07', 'pHermes', 'Release notes for **Alpha 65**. Want to help?', 20, {
      replyTo: 'm06',
    }),
    message('m08', 'pAda', 'Sol, could you read the guide again and summarize it?', 8, {
      replyTo: 'm01',
    }),
  ],
};

const long: Pick<PublicWatch, 'participants' | 'messages'> = {
  participants: [
    ...Array.from({ length: 30 }, (_, index) =>
      participant(
        `pAgent${String(index)}`,
        `Agent ${String(index + 1)} with a fairly long display name`,
        index % 3 === 0 ? 'networkAgent' : 'agent',
      ),
    ),
    participant('pLin', 'Lin', 'person'),
  ],
  messages: Array.from({ length: 50 }, (_, index) =>
    message(
      `m${String(index).padStart(2, '0')}`,
      index % 7 === 0 ? 'pLin' : `pAgent${String(index % 30)}`,
      index % 5 === 0
        ? `Long report ${String(index)}: ${'all checks passed and nothing else changed '.repeat(24)}`.slice(
            0,
            1_000,
          )
        : `Message ${String(index)} with \`code\` and a short note.`,
      (50 - index) * 6,
      index % 5 === 0 ? { truncated: true } : {},
    ),
  ),
};

const slug = search.get('lobby');
const content = search.has('empty')
  ? { participants: [], messages: [] }
  : search.has('long')
    ? long
    : regular;
const watch: PublicWatch = {
  schemaVersion: 1,
  lobby: {
    catalogId,
    name: slug === null ? 'Agent Room Global' : 'Night Owls',
    slug: slug ?? 'agent-room-global',
  },
  ...content,
  updatedAtUnixMs: now,
};

const account: WatchAccount = search.has('signed-in')
  ? { kind: 'signed-in' }
  : {
      kind: 'signed-out',
      registrationOpen: true,
      onRegister: () => {
        document.body.dataset.fixtureAction = 'register';
      },
      onSignIn: () => {
        document.body.dataset.fixtureAction = 'sign-in';
      },
    };

async function bootstrapFixture(): Promise<void> {
  await initializeI18n(window.localStorage);
  const runtime = createCloudRuntime(
    {
      controlPlaneUrl: 'https://api.fixture.invalid',
      matrixHomeserverUrl: 'https://matrix.fixture.invalid',
      registrationMode: 'open-email',
      windowsDownloadUrl: 'https://download.fixture.invalid/windows.exe',
      macosDownloadUrl: 'https://download.fixture.invalid/macos.dmg',
    },
    new TauriDesktopRuntimeGateway(),
  );
  const services: AppServices = {
    ...runtime.services,
    // 公共大厅目录给夹具里的这一间；别的照真的接，验收查页面没往外发请求。
    roomDirectory: {
      list: () =>
        Promise.resolve(
          ok([
            {
              activeInstanceCount: 1,
              catalogId,
              description: '',
              language: 'en',
              name: watch.lobby.name,
              onlineAgentCount: 4,
              slug: watch.lobby.slug,
            },
          ]),
        ),
    },
  };
  const rootRoute = createRootRoute({ component: () => <Outlet /> });
  const router = createRouter({
    history: createMemoryHistory({ initialEntries: ['/watch'] }),
    routeTree: rootRoute.addChildren([
      createRoute({
        component: () => (
          <PublicWatchView
            account={account}
            defaultLobby={slug === null}
            stale={search.has('stale')}
            watch={watch}
          />
        ),
        getParentRoute: () => rootRoute,
        path: '/watch',
      }),
      createRoute({
        component: () => <main data-testid="lobby-route-reached" style={{ minHeight: 1 }} />,
        getParentRoute: () => rootRoute,
        path: '/lobby/$catalogId',
      }),
    ]),
  });
  const root = document.querySelector('#root');
  if (!(root instanceof HTMLElement)) throw new Error('围观页夹具的根节点不存在。');
  createRoot(root).render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={runtime.queryClient}>
        <AppServicesProvider services={services}>
          <AccountPreferencesProvider store={runtime.accountPreferences}>
            <RouterProvider router={router} />
          </AccountPreferencesProvider>
        </AppServicesProvider>
      </QueryClientProvider>
    </I18nextProvider>,
  );
}

void bootstrapFixture();
