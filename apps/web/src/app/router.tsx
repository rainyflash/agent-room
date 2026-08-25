import { createRootRoute, createRoute, createRouter, redirect } from '@tanstack/react-router';
import { lazy, Suspense } from 'react';

import { RootLayout } from '@/app/root-layout';
import { RouteUnavailable } from '@/app/route-unavailable';
import { LobbyStateBoundary } from '@/features/lobby/ui/lobby-state-boundary';
import { ConnectionPage } from '@/features/session/ui/connection-page';
import { useSession } from '@/features/session/ui/session-provider';
import {
  contextIdentifierSchema,
  lobbySearchWithAgent,
  lobbySearchWithMessage,
  normalizeConnectSearch,
  normalizeLobbySearch,
  routeIdentifierSchema,
} from '@/shared/routing/route-state';

const LobbyPage = lazy(async () => {
  const module = await import('@/features/lobby/ui/lobby-page');
  return { default: module.LobbyPage };
});

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
  const { snapshot } = useSession();
  const { catalogId, roomId } = lobbyInstanceRoute.useParams();
  const search = lobbyInstanceRoute.useSearch();
  const navigate = lobbyInstanceRoute.useNavigate();
  const valid =
    routeIdentifierSchema.safeParse(catalogId).success &&
    contextIdentifierSchema.safeParse(roomId).success;
  if (!valid) {
    return <RouteUnavailable invalid routeLabel={`/lobby/${catalogId}/instance/${roomId}`} />;
  }
  return (
    <Suspense
      fallback={<LobbyStateBoundary onRetry={() => undefined} state={{ kind: 'loading' }} />}
    >
      <LobbyPage
        catalogId={catalogId}
        onEnterRoom={(nextCatalogId, nextRoomId) => {
          void navigate({
            params: { catalogId: nextCatalogId, roomId: nextRoomId },
            search: {},
            to: '/lobby/$catalogId/instance/$roomId',
          });
        }}
        onExitRoom={() => {
          void navigate({ to: '/connect' });
        }}
        onSelectedAgentChange={(agentId) => {
          void navigate({
            replace: true,
            search: (previous) => lobbySearchWithAgent(previous, agentId),
          });
        }}
        onSelectedMessageChange={(messageId) => {
          void navigate({
            replace: true,
            search: (previous) => lobbySearchWithMessage(previous, messageId),
          });
        }}
        principal={snapshot.context.principal}
        roomId={roomId}
        selectedAgentId={search.agent ?? null}
        selectedMessageId={search.message ?? null}
      />
    </Suspense>
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
