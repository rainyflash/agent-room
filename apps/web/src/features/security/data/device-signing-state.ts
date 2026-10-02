import { useSyncExternalStore } from 'react';

import type {
  DeviceSigningState,
  DeviceSigningStatus,
} from '@/shared/matrix/device-signing-status';

/** 这台设备的自动签名走到哪一步了（准备中、就绪、出错）。 */
export function useDeviceSigningState(status: DeviceSigningStatus): DeviceSigningState {
  return useSyncExternalStore(status.subscribe, status.getSnapshot);
}
