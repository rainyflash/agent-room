import {
  createRootRoute,
  createRoute,
  createRouter,
  lazyRouteComponent,
} from '@tanstack/react-router';
import { type ComponentProps, type ComponentType, lazy, Suspense, useCallback } from 'react';

import { RootLayout } from '@/app/root-layout';
import { RouteUnavailable } from '@/app/route-unavailable';
import { LobbyStateBoundary } from '@/features/lobby/ui/lobby-state-boundary';
import { PublicLobbyEntryBoundary } from '@/features/lobby-entry/ui/public-lobby-entry-boundary';
import { ConnectionPage } from '@/features/session/ui/connection-page';
import { LandingPage } from '@/features/landing/ui/landing-page';
import { useSession } from '@/features/session/ui/session-provider';
import {
  contextIdentifierSchema,
  lobbySearchWithAgent,
  lobbySearchWithDirectSession,
  lobbySearchWithMessage,
  lobbySearchWithView,
  lobbySearchWithRoomPanel,
  normalizeConnectSearch,
  normalizeLobbySearch,
  normalizeWorkspaceSearch,
  routeIdentifierSchema,
} from '@/shared/routing/route-state';

// 首页与连接页随入口一起加载；其余页面打开时再下载，入口包小一些，冷启动更快。
// 直接挂在路由上的页面由路由先加载好再渲染；放在边界组件里的页面自己带 Suspense。
function lazyPage<Props extends object>(
  load: () => Promise<ComponentType<Props>>,
): ComponentType<Props> {
  const Page = lazy(async () => ({ default: await load() }));
  return function LazyPage(props: Props) {
    return (
      <Suspense fallback={null}>
        <Page {...(props as ComponentProps<typeof Page>)} />
      </Suspense>
    );
  };
}

const ApplicationAboutPage = lazyRouteComponent(
  () => import('@/features/updates/ui/application-about-page'),
  'ApplicationAboutPage',
);
const InboxPage = lazyPage(async () => (await import('@/features/inbox/ui/inbox-page')).InboxPage);
const SecurityPage = lazyPage(
  async () => (await import('@/features/security/ui/security-page')).SecurityPage,
);
const GuidePage = lazyRouteComponent(() => import('@/features/guide/ui/guide-page'), 'GuidePage');
const OnboardingPage = lazyRouteComponent(
  () => import('@/features/onboarding/ui/onboarding-page'),
  'OnboardingPage',
);
const RoomDirectoryPage = lazyPage(
  async () => (await import('@/features/room-directory/ui/room-directory-page')).RoomDirectoryPage,
);
const AccountWorkspacePage = lazyPage(
  async () => (await import('@/features/workspace/ui/account-workspace-page')).AccountWorkspacePage,
);

const LobbyPage = lazy(async () => {
  const module = await import('@/features/lobby/ui/lobby-page');
  return { default: module.LobbyPage };
});

const rootRoute = createRootRoute({ component: RootLayout });
const aboutRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/about',
  component: ApplicationAboutPage,
});

const guideRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/guide',
  component: GuidePage,
});
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

const workspaceRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/workspace',
  validateSearch: normalizeWorkspaceSearch,
  component: WorkspaceBoundary,
});

const roomDirectoryRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/rooms',
  component: RoomDirectoryBoundary,
});
const inboxRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/inbox',
  component: InboxBoundary,
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
  aboutRoute,
  guideRoute,
  indexRoute,
  connectRoute,
  onboardingRoute,
  workspaceRoute,
  roomDirectoryRoute,
  inboxRoute,
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
  const navigate = lobbyRoute.useNavigate();
  const onConnectionRequired = useCallback(() => {
    void navigate({ replace: true, to: '/connect' });
  }, [navigate]);
  const onEntered = useCallback(
    (target: { readonly catalogId: string; readonly matrixRoomId: string }) => {
      void navigate({
        params: { catalogId: target.catalogId, roomId: target.matrixRoomId },
        replace: true,
        search: {},
        to: '/lobby/$catalogId/instance/$roomId',
      });
    },
    [navigate],
  );
  if (!routeIdentifierSchema.safeParse(catalogId).success) {
    return <RouteUnavailable invalid routeLabel={`/lobby/${catalogId}`} />;
  }
  return (
    <PublicLobbyEntryBoundary
      catalogId={catalogId}
      onConnectionRequired={onConnectionRequired}
      onEntered={onEntered}
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
        view={search.view ?? 'space'}
        onOpenRoomPanel={(view) => {
          void navigate({ search: (previous) => lobbySearchWithRoomPanel(previous, view) });
        }}
        onViewChange={(view) => {
          void navigate({ search: (previous) => lobbySearchWithView(previous, view) });
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

function WorkspaceBoundary() {
  const { snapshot } = useSession();
  const search = workspaceRoute.useSearch();
  const navigate = workspaceRoute.useNavigate();
  const principal = snapshot.context.principal;
  if (snapshot.context.controlStatus !== 'ready' || principal === null) {
    return <ConnectionPage />;
  }
  return (
    <AccountWorkspacePage
      onSelectAgent={(agentId) => {
        void navigate({ replace: true, search: { agent: agentId }, to: '/workspace' });
      }}
      principal={principal}
      selectedAgentId={search.agent ?? null}
    />
  );
}

function RoomDirectoryBoundary() {
  const { snapshot } = useSession();
  if (snapshot.context.controlStatus !== 'ready' || snapshot.context.principal === null) {
    return <ConnectionPage />;
  }
  return <RoomDirectoryPage />;
}

function InboxBoundary() {
  const { snapshot } = useSession();
  return snapshot.context.controlStatus === 'ready' && snapshot.context.principal !== null ? (
    <InboxPage />
  ) : (
    <ConnectionPage />
  );
}

function SettingsBoundary() {
  const { snapshot } = useSession();
  const { section } = settingsRoute.useParams();
  const navigate = settingsRoute.useNavigate();
  const valid = routeIdentifierSchema.safeParse(section).success;
  if (!valid || section !== 'security') {
    return <RouteUnavailable invalid={!valid} routeLabel={`/settings/${section}`} />;
  }
  if (snapshot.context.controlStatus !== 'ready' || snapshot.context.connection === null) {
    return <ConnectionPage />;
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
  // eslint-disable-next-line @typescript-eslint/consistent-type-definitions -- 路由注册依赖接口声明合并，类型别名不能替代。
  interface Register {
    router: typeof router;
  }
}
