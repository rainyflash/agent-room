import { createContext, useContext, type PropsWithChildren } from 'react';

import {
  runtimeWriteAvailability,
  type RuntimeWriteAvailability,
} from '@/features/updates/domain/runtime-compatibility';

export type RuntimeCompatibility = {
  readonly applyUpdate: () => Promise<void>;
  readonly updateWaiting: boolean;
  readonly writes: RuntimeWriteAvailability;
};

const compatibleRuntime: RuntimeCompatibility = Object.freeze({
  applyUpdate: () => Promise.resolve(),
  updateWaiting: false,
  writes: runtimeWriteAvailability({ online: true, updateWaiting: false }),
});

const RuntimeCompatibilityContext = createContext<RuntimeCompatibility>(compatibleRuntime);

export function RuntimeCompatibilityBoundary({
  children,
  value,
}: PropsWithChildren<{ readonly value: RuntimeCompatibility }>) {
  return (
    <RuntimeCompatibilityContext.Provider value={value}>
      {children}
    </RuntimeCompatibilityContext.Provider>
  );
}

export function useRuntimeCompatibility(): RuntimeCompatibility {
  return useContext(RuntimeCompatibilityContext);
}
