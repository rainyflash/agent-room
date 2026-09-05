import { invoke } from '@tauri-apps/api/core';
import { z } from 'zod';

import {
  storedMatrixSessionSchema,
  type MatrixSessionVault,
  type StoredMatrixSession,
} from '@/features/session/domain/matrix-session-vault';
import type { SessionFailure } from '@/features/session/domain/session';
import { err, ok, type Result } from '@/shared/result';
import { normalizeCommandFailure } from '@/shared/desktop/command-failure';

type MatrixVaultCommand =
  'desktop_load_matrix_session' | 'desktop_save_matrix_session' | 'desktop_clear_matrix_session';

export type MatrixVaultInvoke = (
  command: MatrixVaultCommand,
  arguments_: Record<string, unknown>,
) => Promise<unknown>;

const voidResponse = z
  .null()
  .or(z.undefined())
  .transform(() => undefined);

/** 命令只允许操作 Matrix 人类会话，不能指定任意凭据名称或文件。 */
export class TauriMatrixSessionVault implements MatrixSessionVault {
  constructor(private readonly call: MatrixVaultInvoke = invoke) {}

  load(): Promise<Result<StoredMatrixSession | null, SessionFailure>> {
    return this.request('desktop_load_matrix_session', {}, storedMatrixSessionSchema.nullable());
  }

  save(session: StoredMatrixSession): Promise<Result<void, SessionFailure>> {
    return this.request('desktop_save_matrix_session', { session }, voidResponse);
  }

  clear(): Promise<Result<void, SessionFailure>> {
    return this.request('desktop_clear_matrix_session', {}, voidResponse);
  }

  private async request<TValue>(
    command: MatrixVaultCommand,
    arguments_: Record<string, unknown>,
    schema: z.ZodType<TValue>,
  ): Promise<Result<TValue, SessionFailure>> {
    try {
      const parsed = schema.safeParse(await this.call(command, arguments_));
      return parsed.success
        ? ok(parsed.data)
        : err(vaultFailure('desktop.matrix_session.vault_invalid_response', false));
    } catch (error: unknown) {
      const normalized = normalizeCommandFailure(error, 'desktop.matrix_session.vault_unavailable');
      return err(vaultFailure(normalized.code, normalized.retryable));
    }
  }
}

function vaultFailure(code: string, retryable: boolean): SessionFailure {
  return { boundary: 'matrix', code, offline: false, retryable };
}
