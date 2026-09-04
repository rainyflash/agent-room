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

import type { PublicRoomSummary } from '@/features/room-directory/domain/public-room-directory';
import { RoomDirectoryView } from '@/features/room-directory/ui/room-directory-page';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

const rooms: readonly PublicRoomSummary[] = [
  {
    activeInstanceCount: 1,
    catalogId: '0198b601-77a2-7f41-b4f4-940f291951b8',
    description:
      'Observe live Agent state, exchange public messages, and open verified details only when you choose.',
    language: 'en',
    name: 'Default public lobby',
    onlineAgentCount: 2,
    slug: 'default-public',
  },
  {
    activeInstanceCount: 0,
    catalogId: '0198b601-77a3-74f1-b4f4-940f291951b9',
    description: '面向中文 Agent 的公共协作空间。',
    language: 'zh-CN',
    name: '中文公共大厅',
    onlineAgentCount: 0,
    slug: 'zh-public',
  },
];

const rootRoute = createRootRoute({ component: () => <Outlet /> });
const directoryRoute = createRoute({
  component: () => (
    <RoomDirectoryView
      failureCode={null}
      loading={false}
      onRefresh={() => undefined}
      rooms={rooms}
    />
  ),
  getParentRoute: () => rootRoute,
  path: '/rooms',
});
const workspaceRoute = createRoute({
  component: () => null,
  getParentRoute: () => rootRoute,
  path: '/workspace',
});
const settingsRoute = createRoute({
  component: () => null,
  getParentRoute: () => rootRoute,
  path: '/settings/$section',
});
const lobbyRoute = createRoute({
  component: () => <main data-testid="lobby-route-reached" style={{ minHeight: 1 }} />,
  getParentRoute: () => rootRoute,
  path: '/lobby/$catalogId',
});
const router = createRouter({
  history: createMemoryHistory({ initialEntries: ['/rooms'] }),
  routeTree: rootRoute.addChildren([directoryRoute, workspaceRoute, settingsRoute, lobbyRoute]),
});

async function bootstrapFixture(): Promise<void> {
  await initializeI18n(window.localStorage, ['en']);
  const root = document.querySelector('#root');
  if (!(root instanceof HTMLElement)) {
    throw new Error('房间目录测试根节点不存在。');
  }
  createRoot(root).render(
    <I18nextProvider i18n={i18n}>
      <RouterProvider router={router} />
    </I18nextProvider>,
  );
}

void bootstrapFixture();
