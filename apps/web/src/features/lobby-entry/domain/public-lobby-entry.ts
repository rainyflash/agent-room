import { z } from 'zod';

import type { Result } from '@/shared/result';
import { matrixRoomIdSchema, uuidV7Schema } from '@/shared/validation/identifiers';

export const publicLobbyEntryTargetSchema = z
  .object({
    catalogId: uuidV7Schema,
    roomInstanceId: uuidV7Schema,
    matrixRoomId: matrixRoomIdSchema,
  })
  .strict();

export const publicLobbyRouteTargetSchema = z
  .object({
    catalogId: uuidV7Schema,
    matrixRoomId: matrixRoomIdSchema,
  })
  .strict();

export type PublicLobbyEntryTarget = z.infer<typeof publicLobbyEntryTargetSchema>;
export type PublicLobbyRouteTarget = z.infer<typeof publicLobbyRouteTargetSchema>;

export type PublicLobbyEntryFailure = {
  readonly code: string;
  readonly retryable: boolean;
};

export type PublicLobbyEntryGateway = {
  resolve(catalogId: string): Promise<Result<PublicLobbyEntryTarget, PublicLobbyEntryFailure>>;
};

export type PublicLobbyMatrixGateway = {
  join(matrixRoomId: string): Promise<Result<void, PublicLobbyEntryFailure>>;
};
