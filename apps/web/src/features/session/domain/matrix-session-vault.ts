import { z } from 'zod';

import type { SessionFailure } from './session';
import type { Result } from '@/shared/result';

const boundedValue = (maximum: number) =>
  z
    .string()
    .min(1)
    .max(maximum)
    .refine((value) => !/[\p{Cc}\s]/u.test(value));

export const storedMatrixSessionSchema = z
  .object({
    accessToken: boundedValue(4_096),
    deviceId: boundedValue(255),
    refreshToken: boundedValue(4_096).optional(),
    userId: boundedValue(255).regex(/^@[^:]+:.+$/u),
    version: z.literal(1),
  })
  .strict();

export type StoredMatrixSession = z.output<typeof storedMatrixSessionSchema>;

/** 会话持久化策略由组合根选择，Matrix 连接无需知道浏览器或操作系统。 */
export type MatrixSessionVault = {
  load(): Promise<Result<StoredMatrixSession | null, SessionFailure>>;
  save(session: StoredMatrixSession): Promise<Result<void, SessionFailure>>;
  clear(): Promise<Result<void, SessionFailure>>;
};
