import { z } from 'zod';

import type { Result } from '@/shared/result';

export type AccountFailure = {
  readonly code: string;
  readonly correlationId?: string;
  readonly retryable: boolean;
};

/** 下载我的数据：服务器给的 JSON，排好版存成一个文件。 */
export type AccountExport = {
  readonly fileName: string;
  readonly json: string;
};

export type AccountGateway = {
  /** 资料、电脑、Agent、房间、举报和文件清单；消息不在里面。 */
  exportData(): Promise<Result<AccountExport, AccountFailure>>;
  /**
   * 删除账户。要几分钟内登录过（否则回 `authentication.reauthentication_required`）；同一个编号重试只删一次。
   * 成功以后服务器已经让这次登录失效，界面要接着退出登录、清掉本机数据。
   */
  requestDeletion(idempotencyKey: string): Promise<Result<void, AccountFailure>>;
};

/** 服务器要求的确认词，界面上让人照着输入。 */
export const ACCOUNT_DELETION_CONFIRMATION = 'DELETE';

export const REAUTHENTICATION_REQUIRED = 'authentication.reauthentication_required';

export const accountExportSchema = z.looseObject({
  schemaVersion: z.number().int().positive(),
  generatedAtUnixMs: z.number().int().nonnegative(),
  data: z.record(z.string(), z.unknown()),
});

export const accountDeletionStartedSchema = z.looseObject({
  receipt: z.string().min(1),
  progress: z.looseObject({ jobId: z.string().min(1), stage: z.string().min(1) }),
});
