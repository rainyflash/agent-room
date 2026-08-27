import { createRootRoute, createRoute, createRouter } from '@tanstack/react-router';
import { lazy, Suspense } from 'react';

import { RootLayout } from '@/app/root-layout';
import { RouteUnavailable } from '@/app/route-unavailable';
import { LobbyStateBoundary } from '@/features/lobby/ui/lobby-state-boundary';
import { SecurityPage } from '@/features/security/ui/security-page';
import { ConnectionPage } from '@/features/session/ui/connection-page';
import { LandingPage } from '@/features/landing/ui/landing-page';
import { OnboardingPage } from '@/features/onboarding/ui/onboarding-page';
import { useSession } from '@/features/session/ui/session-provider';
import {
  contextIdentifierSchema,
  lobbySearchWithAgent,
  lobbySearchWithDirectSession,
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
  component: LandingPage,
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

const onboardingRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/onboarding',
  component: OnboardingPage,
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
  onboardingRoute,
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
        onOpenSecurity={() => {
          void navigate({ params: { section: 'security' }, to: '/settings/$section' });
        }}
        onSelectedAgentChange={(agentId) => {
          void navigate({
            replace: true,
            search: (previous) => lobbySearchWithAgent(previous, agentId),
          });
        }}
        onSelectedDirectSessionChange={(catalogId) => {
          void navigate({
            replace: true,
            search: (previous) => lobbySearchWithDirectSession(previous, catalogId),
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
        selectedDirectSessionId={search.direct ?? null}
        selectedMessageId={search.message ?? null}
      />
    </Suspense>
  );
}

function SettingsBoundary() {
  const { section } = settingsRoute.useParams();
  const navigate = settingsRoute.useNavigate();
  const valid = routeIdentifierSchema.safeParse(section).success;
  if (!valid || section !== 'security') {
    return <RouteUnavailable invalid={!valid} routeLabel={`/settings/${section}`} />;
  }
  return (
    <SecurityPage
      onBack={() => {
        if (window.history.length > 1) {
          window.history.back();
          return;
        }
        void navigate({ to: '/connect' });
      }}
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
