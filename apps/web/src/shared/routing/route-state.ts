import { z } from 'zod';
import type { RoomPageLayout, RoomWorkspaceView } from '@/features/lobby/domain/workspace-view';

import { safeInternalPath } from '@/shared/browser/window-browser-gateway';

export const routeIdentifierSchema = z
  .string()
  .trim()
  .min(1)
  .max(255)
  .regex(/^[^\s\\/?#]+$/u);

export const contextIdentifierSchema = z
  .string()
  .trim()
  .min(1)
  .max(512)
  .regex(/^[^\s\\?#]+$/u);

export type ConnectSearch = {
  readonly authentication?: 'expired' | 'failed';
  readonly returnTo?: string;
};

export type LobbySearch = {
  readonly view?: Exclude<RoomWorkspaceView, 'space'>;
  readonly agent?: string;
  readonly direct?: string;
  readonly directory?: 'open';
  /** 房间页的版面。不写时看这台设备记着的（`features/lobby/application/room-page-layout.ts`）。 */
  readonly layout?: RoomPageLayout;
  readonly message?: string;
};

export type WorkspaceSearch = {
  readonly agent?: string;
};

export function normalizeConnectSearch(search: Record<string, unknown>): ConnectSearch {
  const returnTo = typeof search.returnTo === 'string' ? safeInternalPath(search.returnTo) : null;
  return {
    ...(returnTo === null ? {} : { returnTo }),
    ...(search.authentication === 'expired' || search.authentication === 'failed'
      ? { authentication: search.authentication }
      : {}),
  };
}

export function normalizeLobbySearch(search: Record<string, unknown>): LobbySearch {
  const agent = contextIdentifierSchema.safeParse(search.agent);
  const direct = contextIdentifierSchema.safeParse(search.direct);
  const message = contextIdentifierSchema.safeParse(search.message);
  return {
    ...(agent.success ? { agent: agent.data } : {}),
    ...(direct.success ? { direct: direct.data } : {}),
    ...(search.directory === 'open' ? { directory: 'open' as const } : {}),
    ...(search.layout === 'conversation' || search.layout === 'scene'
      ? { layout: search.layout }
      : {}),
    ...(message.success ? { message: message.data } : {}),
    ...(search.view === 'resources' || search.view === 'conversation' ? { view: search.view } : {}),
  };
}

export function normalizeWorkspaceSearch(search: Record<string, unknown>): WorkspaceSearch {
  const agent = contextIdentifierSchema.safeParse(search.agent);
  return agent.success ? { agent: agent.data } : {};
}

/** 换哪种选择都留着的：房间目录开没开、地址里写的版面。 */
function kept(search: LobbySearch): LobbySearch {
  return {
    ...(search.directory === undefined ? {} : { directory: search.directory }),
    ...(search.layout === undefined ? {} : { layout: search.layout }),
  };
}

/**
 * 选中或放开一个 Agent。场景里选中 Agent 会收起对话面板，所以别的选择一起清掉；
 * 对话版面里 Agent 的详情在右边一栏，对话、私聊和正在看的那条消息都留着。
 */
export function lobbySearchWithAgent(
  search: LobbySearch,
  agentId: string | null,
  layout: RoomPageLayout = 'scene',
): LobbySearch {
  const keepsContext = agentId === null || layout === 'conversation';
  return {
    ...(agentId === null ? {} : { agent: agentId }),
    ...(keepsContext && search.view !== undefined ? { view: search.view } : {}),
    ...(keepsContext && search.direct !== undefined ? { direct: search.direct } : {}),
    ...kept(search),
    ...(keepsContext && search.message !== undefined ? { message: search.message } : {}),
  };
}

export function lobbySearchWithDirectSession(
  search: LobbySearch,
  catalogId: string | null,
): LobbySearch {
  return {
    ...(catalogId === null ? {} : { direct: catalogId }),
    ...kept(search),
  };
}

export function lobbySearchWithMessage(search: LobbySearch, messageId: string | null): LobbySearch {
  return {
    ...(messageId === null && search.agent !== undefined ? { agent: search.agent } : {}),
    ...(search.direct === undefined ? {} : { direct: search.direct }),
    ...kept(search),
    ...(messageId === null ? {} : { message: messageId }),
    ...(search.view === undefined ? {} : { view: search.view }),
  };
}

export function lobbySearchWithView(search: LobbySearch, view: RoomWorkspaceView): LobbySearch {
  if (view === 'space')
    return {
      ...(search.agent === undefined ? {} : { agent: search.agent }),
      ...kept(search),
    };
  return normalizeLobbySearch({ ...search, view });
}

export function lobbySearchWithRoomPanel(
  search: LobbySearch,
  view: 'conversation' | 'resources',
): LobbySearch {
  return { view, ...kept(search) };
}

/**
 * 换版面，写进地址。换到场景时回到画面本身：对话面板、私聊和正在看的消息都收起来；
 * 换到对话时这些都留着。
 */
export function lobbySearchWithLayout(search: LobbySearch, layout: RoomPageLayout): LobbySearch {
  if (layout === 'scene')
    return {
      ...(search.agent === undefined ? {} : { agent: search.agent }),
      ...(search.directory === undefined ? {} : { directory: search.directory }),
      layout,
    };
  return { ...search, layout };
}
