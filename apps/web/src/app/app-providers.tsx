import { useMemo } from 'react';

import { CloudAppProviders } from '@/app/web-app-providers';
import { TauriDesktopRuntimeGateway } from '@/features/desktop/adapters/tauri-desktop-runtime-gateway';
import type { DesktopRuntimeGateway } from '@/features/desktop/domain/desktop-runtime';
import type { RuntimeConfig } from '@/shared/config/runtime-config';

export type AppProvidersProps = {
  readonly config: RuntimeConfig;
  readonly localRuntime?: DesktopRuntimeGateway;
};

export function AppProviders({ config, localRuntime }: AppProvidersProps) {
  const nativeRuntime = useMemo(() => new TauriDesktopRuntimeGateway(), []);
  return <CloudAppProviders config={config} localRuntime={localRuntime ?? nativeRuntime} />;
}
