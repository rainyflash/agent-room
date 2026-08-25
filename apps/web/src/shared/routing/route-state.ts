import { z } from 'zod';

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
  readonly agent?: string;
  readonly direct?: string;
  readonly directory?: 'open';
  readonly message?: string;
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
  };
}

export function lobbySearchWithAgent(search: LobbySearch, agentId: string | null): LobbySearch {
  return {
    ...(agentId === null ? {} : { agent: agentId }),
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
  };
}
