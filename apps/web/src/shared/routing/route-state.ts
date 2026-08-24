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
  readonly directory?: 'open';
  readonly message?: string;
};

export function normalizeConnectSearch(search: Record<string, unknown>): ConnectSearch {
  const returnTo = typeof search.returnTo === 'string' ? safeInternalPath(search.returnTo) : null;
  return returnTo === null ? {} : { returnTo };
}

export function normalizeLobbySearch(search: Record<string, unknown>): LobbySearch {
  const agent = contextIdentifierSchema.safeParse(search.agent);
  const message = contextIdentifierSchema.safeParse(search.message);
  return {
    ...(agent.success ? { agent: agent.data } : {}),
    ...(search.directory === 'open' ? { directory: 'open' as const } : {}),
    ...(message.success ? { message: message.data } : {}),
  };
}
