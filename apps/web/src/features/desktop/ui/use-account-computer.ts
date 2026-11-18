import { useEffect, useState } from 'react';

import { useOptionalAppServices } from '@/app/app-services';
import type { ProductDevice } from '@/features/security/domain/access-management';

/** 这个账号有没有装着 Agent Room 的电脑；`unknown` 是还在问。 */
export type AccountComputer = 'unknown' | 'none' | 'some';

function isComputer(device: ProductDevice): boolean {
  return device.revokedAtUnixMs === null && device.platform !== 'web';
}

/**
 * 网页端接入 Agent 时看一眼这个账号授权过电脑没有：有的话不再把“先装桌面应用”当成头一步。
 * 问不到（没有服务、请求没成）当成没有，照样给下载。
 */
export function useAccountComputer(enabled: boolean): AccountComputer {
  const gateway = useOptionalAppServices()?.accessManagement ?? null;
  const [answer, setAnswer] = useState<AccountComputer>('unknown');
  useEffect(() => {
    if (!enabled || gateway === null) return undefined;
    let active = true;
    void gateway.listProductDevices().then(
      (result) => {
        if (active) setAnswer(result.ok && result.value.some(isComputer) ? 'some' : 'none');
      },
      () => {
        if (active) setAnswer('none');
      },
    );
    return () => {
      active = false;
    };
  }, [enabled, gateway]);
  return !enabled || gateway === null ? 'none' : answer;
}
