import { z } from 'zod';

import type { Result } from '@/shared/result';

/**
 * 提醒设置（specs/redesign/reminders.md 4R-3）：邮件提醒的开关跟着账户走，存在控制面。
 * `available` 是这台服务器开没开邮件提醒；没开时设置里不显示这一行。
 */
export type ReminderSettings = {
  readonly email: { readonly enabled: boolean; readonly available: boolean };
};

export type ReminderSettingsFailure = {
  readonly code: string;
  readonly correlationId?: string;
  readonly retryable: boolean;
};

export type ReminderSettingsGateway = {
  read(): Promise<Result<ReminderSettings, ReminderSettingsFailure>>;
  setEmail(enabled: boolean): Promise<Result<ReminderSettings, ReminderSettingsFailure>>;
};

/** 新接口的回答：多一个、少一个字段都不认。 */
export const reminderSettingsSchema = z
  .object({
    email: z.object({ enabled: z.boolean(), available: z.boolean() }).strict(),
  })
  .strict();
