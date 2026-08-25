import { useEffect, useMemo, useState, type PropsWithChildren } from 'react';
import { useRegisterSW } from 'virtual:pwa-register/react';

import { runtimeWriteAvailability } from '@/features/updates/domain/runtime-compatibility';
import {
  RuntimeCompatibilityBoundary,
  type RuntimeCompatibility,
} from '@/features/updates/ui/runtime-compatibility-context';

export function RuntimeCompatibilityProvider({ children }: PropsWithChildren) {
  const {
    needRefresh: [needRefresh],
    updateServiceWorker,
  } = useRegisterSW();
  const [online, setOnline] = useState(() => window.navigator.onLine);
  // 一旦发现等待中的版本，本页永久进入只读；不能靠关闭提示重新放行旧协议。
  const [updateWaiting, setUpdateWaiting] = useState(false);

  useEffect(() => {
    if (needRefresh) {
      setUpdateWaiting(true);
    }
  }, [needRefresh]);

  useEffect(() => {
    const handleOnline = (): void => {
      setOnline(true);
    };
    const handleOffline = (): void => {
      setOnline(false);
    };
    window.addEventListener('online', handleOnline);
    window.addEventListener('offline', handleOffline);
    return () => {
      window.removeEventListener('online', handleOnline);
      window.removeEventListener('offline', handleOffline);
    };
  }, []);

  const value = useMemo<RuntimeCompatibility>(
    () => ({
      applyUpdate: async () => {
        await updateServiceWorker(true);
      },
      updateWaiting,
      writes: runtimeWriteAvailability({ online, updateWaiting }),
    }),
    [online, updateServiceWorker, updateWaiting],
  );
  return <RuntimeCompatibilityBoundary value={value}>{children}</RuntimeCompatibilityBoundary>;
}
