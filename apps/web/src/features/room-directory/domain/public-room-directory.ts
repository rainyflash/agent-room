import { z } from 'zod';

import type { Result } from '@/shared/result';
import { uuidV7Schema } from '@/shared/validation/identifiers';

export const publicRoomSummarySchema = z
  .object({
    catalogId: uuidV7Schema,
    slug: z.string().trim().min(1).max(96).nullable(),
    name: z.string().trim().min(1).max(160),
    description: z.string().max(4_000),
    language: z.string().trim().min(1).max(35).nullable(),
    activeInstanceCount: z.number().int().min(0).max(65_535),
    onlineAgentCount: z.number().int().nonnegative(),
  })
  .strict();

export const publicLobbyDirectoryResponseSchema = z
  .object({ lobbies: z.array(publicRoomSummarySchema) })
  .strict();

export type PublicRoomSummary = z.infer<typeof publicRoomSummarySchema>;

export type PublicRoomDirectoryFailure = {
  readonly code: string;
  readonly retryable: boolean;
};

export type PublicRoomDirectoryGateway = {
  list(): Promise<Result<readonly PublicRoomSummary[], PublicRoomDirectoryFailure>>;
};

export function selectPreferredPublicRoom(
  rooms: readonly PublicRoomSummary[],
  preferredLocale: string,
): PublicRoomSummary | null {
  if (rooms.length === 0) return null;
  const normalizedLocale = preferredLocale.trim().toLowerCase();
  const baseLanguage = normalizedLocale.split('-')[0];
  return (
    rooms.find((room) => room.language?.toLowerCase() === normalizedLocale) ??
    rooms.find((room) => room.language?.toLowerCase().split('-')[0] === baseLanguage) ??
    rooms[0] ??
    null
  );
}
