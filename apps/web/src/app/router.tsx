import { createRootRoute, createRoute, createRouter, redirect } from '@tanstack/react-router';

import { RootLayout } from '@/app/root-layout';
import { RouteUnavailable } from '@/app/route-unavailable';
import { ConnectionPage } from '@/features/session/ui/connection-page';
import {
  contextIdentifierSchema,
  normalizeConnectSearch,
  normalizeLobbySearch,
  routeIdentifierSchema,
} from '@/shared/routing/route-state';

const rootRoute = createRootRoute({ component: RootLayout });

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/',
  beforeLoad: () => {
    throw redirect({ to: '/connect' });
  },
});

const connectRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/connect',
  validateSearch: normalizeConnectSearch,
  component: ConnectionPage,
});

const lobbyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/lobby/$catalogId',
  validateSearch: normalizeLobbySearch,
  component: LobbyBoundary,
});

const lobbyInstanceRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/lobby/$catalogId/instance/$roomId',
  validateSearch: normalizeLobbySearch,
  component: LobbyInstanceBoundary,
});

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/settings/$section',
  component: SettingsBoundary,
});

const adminRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/admin/$scope',
  component: AdminBoundary,
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  connectRoute,
  lobbyRoute,
  lobbyInstanceRoute,
  settingsRoute,
  adminRoute,
]);

export const router = createRouter({
  routeTree,
  defaultNotFoundComponent: () => (
    <RouteUnavailable invalid routeLabel={window.location.pathname} />
  ),
});

function LobbyBoundary() {
  const { catalogId } = lobbyRoute.useParams();
  return (
    <RouteUnavailable
      invalid={!routeIdentifierSchema.safeParse(catalogId).success}
      routeLabel={`/lobby/${catalogId}`}
    />
  );
}

function LobbyInstanceBoundary() {
  const { catalogId, roomId } = lobbyInstanceRoute.useParams();
  const valid =
    routeIdentifierSchema.safeParse(catalogId).success &&
    contextIdentifierSchema.safeParse(roomId).success;
  return (
    <RouteUnavailable invalid={!valid} routeLabel={`/lobby/${catalogId}/instance/${roomId}`} />
  );
}

function SettingsBoundary() {
  const { section } = settingsRoute.useParams();
  return (
    <RouteUnavailable
      invalid={!routeIdentifierSchema.safeParse(section).success}
      routeLabel={`/settings/${section}`}
    />
  );
}

function AdminBoundary() {
  const { scope } = adminRoute.useParams();
  return (
    <RouteUnavailable
      invalid={!routeIdentifierSchema.safeParse(scope).success}
      routeLabel={`/admin/${scope}`}
    />
  );
}

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router;
  }
}
