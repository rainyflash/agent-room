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

import { RoomDirectoryView } from '@/features/room-directory/ui/room-directory-page';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

/** “你的房间”一节要连控制面，夹具里只放个占位，看的是页面的架子和去发现页的那条路。 */
function MyRoomsPlaceholder() {
  return (
    <section className="my-rooms" data-testid="my-rooms-slot">
      <h2>Your rooms</h2>
    </section>
  );
}

const rootRoute = createRootRoute({ component: () => <Outlet /> });
const directoryRoute = createRoute({
  component: () => <RoomDirectoryView myRooms={<MyRoomsPlaceholder />} />,
  getParentRoute: () => rootRoute,
  path: '/rooms',
});
const exploreRoute = createRoute({
  component: () => <main data-testid="explore-route-reached" style={{ minHeight: 1 }} />,
  getParentRoute: () => rootRoute,
  path: '/explore',
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
  routeTree: rootRoute.addChildren([
    directoryRoute,
    exploreRoute,
    workspaceRoute,
    settingsRoute,
    lobbyRoute,
  ]),
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
