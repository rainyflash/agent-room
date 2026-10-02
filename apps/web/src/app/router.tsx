import {
  createRootRoute,
  createRoute,
  createRouter,
  lazyRouteComponent,
  redirect,
} from '@tanstack/react-router';
import { type ComponentProps, type ComponentType, lazy, Suspense, useCallback } from 'react';

import { RootLayout } from '@/app/root-layout';
import { RouteUnavailable } from '@/app/route-unavailable';
import { LobbyStateBoundary } from '@/features/lobby/ui/lobby-state-boundary';
import { PublicLobbyEntryBoundary } from '@/features/lobby-entry/ui/public-lobby-entry-boundary';
import { ConnectionPage } from '@/features/session/ui/connection-page';
import { LandingPage } from '@/features/landing/ui/landing-page';
import { useSession } from '@/features/session/ui/session-provider';
import { isSettingsSection } from '@/features/settings/ui/settings-sections';
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

const InboxPage = lazyPage(async () => (await import('@/features/inbox/ui/inbox-page')).InboxPage);
const SettingsRoute = lazyPage(
  async () => (await import('@/features/settings/ui/settings-route')).SettingsRoute,
);
const GuidePage = lazyRouteComponent(() => import('@/features/guide/ui/guide-page'), 'GuidePage');
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
// “关于与更新”并进了设置：旧链接转到“设置 → 关于”。
const aboutRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/about',
  beforeLoad: () => {
    // eslint-disable-next-line @typescript-eslint/only-throw-error -- 路由库用抛出 redirect 表示跳转。
    throw redirect({ params: { section: 'about' }, replace: true, to: '/settings/$section' });
  },
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

// 首次使用页已经去掉（界面翻新第 3 步）：旧链接转到“我的 Agent”。
const onboardingRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/onboarding',
  beforeLoad: () => {
    // eslint-disable-next-line @typescript-eslint/only-throw-error -- 路由库用抛出 redirect 表示跳转。
    throw redirect({ replace: true, search: {}, to: '/workspace' });
  },
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

const settingsIndexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/settings',
  beforeLoad: () => {
    // eslint-disable-next-line @typescript-eslint/only-throw-error -- 路由库用抛出 redirect 表示跳转。
    throw redirect({ params: { section: 'general' }, replace: true, to: '/settings/$section' });
  },
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
  settingsIndexRoute,
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
  // 没登录时先去登录，登录完回到这个房间，而不是停在房间列表。
  const onConnectionRequired = useCallback(() => {
    void navigate({ replace: true, search: { returnTo: `/lobby/${catalogId}` }, to: '/connect' });
  }, [catalogId, navigate]);
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
        onExitRoom={() => {
          void navigate({ to: '/rooms' });
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
        void navigate({
          replace: true,
          search: agentId === null ? {} : { agent: agentId },
          to: '/workspace',
        });
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
  if (!isSettingsSection(section)) {
    return (
      <RouteUnavailable
        invalid={!routeIdentifierSchema.safeParse(section).success}
        routeLabel={`/settings/${section}`}
      />
    );
  }
  // 关于、这台电脑不用登录；通用要账户，安全还要连上 Matrix。
  const signedIn =
    snapshot.context.controlStatus === 'ready' && snapshot.context.principal !== null;
  if (section === 'general' && !signedIn) return <ConnectionPage />;
  if (section === 'security' && (!signedIn || snapshot.context.connection === null)) {
    return <ConnectionPage />;
  }
  return <SettingsRoute section={section} />;
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
