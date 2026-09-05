import { z } from 'zod';
import type { RoomWorkspaceView } from '@/features/lobby/domain/workspace-view';

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
  readonly returnTo?: string;
};

export type LobbySearch = {
  readonly view?: Exclude<RoomWorkspaceView, 'space'>;
  readonly agent?: string;
  readonly direct?: string;
  readonly directory?: 'open';
  readonly message?: string;
};

export type WorkspaceSearch = {
  readonly agent?: string;
};

export function normalizeConnectSearch(search: Record<string, unknown>): ConnectSearch {
  const returnTo = typeof search.returnTo === 'string' ? safeInternalPath(search.returnTo) : null;
  return returnTo === null ? {} : { returnTo };
}

export function normalizeLobbySearch(search: Record<string, unknown>): LobbySearch {
  const agent = contextIdentifierSchema.safeParse(search.agent);
  const direct = contextIdentifierSchema.safeParse(search.direct);
  const message = contextIdentifierSchema.safeParse(search.message);
  return {
    ...(agent.success ? { agent: agent.data } : {}),
    ...(direct.success ? { direct: direct.data } : {}),
    ...(search.directory === 'open' ? { directory: 'open' as const } : {}),
    ...(message.success ? { message: message.data } : {}),
    ...(search.view === 'resources' || search.view === 'conversation' ? { view: search.view } : {}),
  };
}

export function normalizeWorkspaceSearch(search: Record<string, unknown>): WorkspaceSearch {
  const agent = contextIdentifierSchema.safeParse(search.agent);
  return agent.success ? { agent: agent.data } : {};
}

export function lobbySearchWithAgent(search: LobbySearch, agentId: string | null): LobbySearch {
  return {
    ...(agentId === null ? {} : { agent: agentId }),
    ...(agentId === null && search.view !== undefined ? { view: search.view } : {}),
    ...(agentId === null && search.direct !== undefined ? { direct: search.direct } : {}),
    ...(search.directory === undefined ? {} : { directory: search.directory }),
    ...(agentId === null && search.message !== undefined ? { message: search.message } : {}),
  };
}

export function lobbySearchWithDirectSession(
  search: LobbySearch,
  catalogId: string | null,
): LobbySearch {
  return {
    ...(catalogId === null ? {} : { direct: catalogId }),
    ...(search.directory === undefined ? {} : { directory: search.directory }),
  };
}

export function lobbySearchWithMessage(search: LobbySearch, messageId: string | null): LobbySearch {
  return {
    ...(messageId === null && search.agent !== undefined ? { agent: search.agent } : {}),
    ...(search.direct === undefined ? {} : { direct: search.direct }),
    ...(search.directory === undefined ? {} : { directory: search.directory }),
    ...(messageId === null ? {} : { message: messageId }),
    ...(search.view === undefined ? {} : { view: search.view }),
  };
}

export function lobbySearchWithView(search: LobbySearch, view: RoomWorkspaceView): LobbySearch {
  if (view === 'space')
    return {
      ...(search.agent === undefined ? {} : { agent: search.agent }),
      ...(search.directory === undefined ? {} : { directory: search.directory }),
    };
  return normalizeLobbySearch({ ...search, view });
}

export function lobbySearchWithRoomPanel(
  search: LobbySearch,
  view: 'conversation' | 'resources',
): LobbySearch {
  return { view, ...(search.directory === undefined ? {} : { directory: search.directory }) };
}
