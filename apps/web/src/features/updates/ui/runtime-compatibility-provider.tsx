import { useEffect, useMemo, useState, type PropsWithChildren } from 'react';
import { useRegisterSW } from 'virtual:pwa-register/react';

import { runtimeWriteAvailability } from '@/features/updates/domain/runtime-compatibility';
import { readRuntimeManifest, currentWriteContract } from '../domain/runtime-manifest';
import { prepareForUpdate } from '../application/update-readiness';
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
  // Dismissing the prompt must not bypass the compatibility check.
  const [updateWaiting, setUpdateWaiting] = useState(false);
  const [contractCompatible, setContractCompatible] = useState(false);

  useEffect(() => {
    if (!updateWaiting || !online) return;
    const abort = new AbortController();
    void readRuntimeManifest(abort.signal)
      .then((manifest) => {
        if (!abort.signal.aborted)
          setContractCompatible(manifest.writeContract === currentWriteContract);
      })
      .catch(() => {
        if (!abort.signal.aborted) setContractCompatible(false);
      });
    return () => {
      abort.abort();
    };
  }, [online, updateWaiting]);

  useEffect(() => {
    const protectDrafts = (event: BeforeUnloadEvent) => {
      if (!prepareForUpdate()) event.preventDefault();
    };
    window.addEventListener('beforeunload', protectDrafts);
    return () => {
      window.removeEventListener('beforeunload', protectDrafts);
    };
  }, []);

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
        if (!prepareForUpdate()) throw new Error('update.draft_unsaved');
        await updateServiceWorker(true);
      },
      updateWaiting,
      writes: runtimeWriteAvailability({ online, updateWaiting, contractCompatible }),
    }),
    [online, updateServiceWorker, updateWaiting, contractCompatible],
  );
  return <RuntimeCompatibilityBoundary value={value}>{children}</RuntimeCompatibilityBoundary>;
}
