import { useQuery } from '@tanstack/react-query';

import type { ReminderSettingsGateway } from '@/features/reminders/domain/reminder-settings';
import { err } from '@/shared/result';

/** 测试和夹具里没有这项服务时不去问；查询本来也不会跑。 */
const unavailable = { code: 'reminders.unavailable', retryable: false } as const;

export const reminderSettingsQueryKey = (principalId: string | null) =>
  ['reminder-settings', principalId] as const;

/** 本人的提醒设置。读不出来（旧控制面没有这个接口、网络不通）时设置里不显示邮件那一行。 */
export function useReminderSettings(
  gateway: ReminderSettingsGateway | undefined,
  principalId: string | null,
) {
  return useQuery({
    enabled: gateway !== undefined,
    queryKey: reminderSettingsQueryKey(principalId),
    queryFn: async () => (gateway === undefined ? err(unavailable) : await gateway.read()),
    retry: false,
    staleTime: 60_000,
  });
}
