import { createRootRoute, createRoute, createRouter } from '@tanstack/react-router';

import { DesktopRootLayout } from '@/app/desktop-root-layout';
import { DesktopConnectionPage } from '@/features/desktop/ui/desktop-connection-page';
import { DesktopLobbyPage } from '@/features/desktop/ui/desktop-lobby-page';
import { normalizeConnectSearch, normalizeLobbySearch } from '@/shared/routing/route-state';

const rootRoute = createRootRoute({ component: DesktopRootLayout });

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/',
  component: DesktopConnectionPage,
});

const connectRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/connect',
  validateSearch: normalizeConnectSearch,
  component: DesktopConnectionPage,
});

const onboardingRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/onboarding',
  component: DesktopConnectionPage,
});

const lobbyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/lobby/$catalogId',
  validateSearch: normalizeLobbySearch,
  component: DesktopLobbyPage,
});

const lobbyInstanceRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/lobby/$catalogId/instance/$roomId',
  validateSearch: normalizeLobbySearch,
  component: DesktopLobbyPage,
});

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/settings/$section',
  component: DesktopConnectionPage,
});

const adminRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/admin/$scope',
  component: DesktopConnectionPage,
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  connectRoute,
  onboardingRoute,
  lobbyRoute,
  lobbyInstanceRoute,
  settingsRoute,
  adminRoute,
]);

export const desktopRouter = createRouter({
  routeTree,
  defaultNotFoundComponent: DesktopConnectionPage,
});
