import type { DeviceSigningState } from '@/shared/matrix/device-signing-status';

import type { MatrixSecurityDevice, MatrixSecuritySnapshot } from './matrix-security';

/**
 * 这台设备：已就绪、正在准备，或者出错可重试。签名全自动（ADR 0011，
 * `specs/device-signing/design.md`），人不用做什么；出错时只给一个“重试”。
 */
export type ThisDeviceState = 'failed' | 'ready' | 'working';

export function thisDeviceState(
  snapshot: Pick<MatrixSecuritySnapshot, 'devices'> | null,
  signing: DeviceSigningState,
): ThisDeviceState {
  // 由你的签名身份签过就能收发：Agent 给不给房间密钥只看这个。自动签名跑完也算，
  // 设备列表可能还没刷新到。
  const current = snapshot?.devices.find((device) => device.current);
  if ((current !== undefined && deviceSignedByYou(current)) || signing.kind === 'ready') {
    return 'ready';
  }
  return signing.kind === 'failed' ? 'failed' : 'working';
}

/** 由你的身份签过名的设备才拿得到加密房间的钥匙。 */
export function deviceSignedByYou(device: MatrixSecurityDevice): boolean {
  return device.trust === 'verified' || device.trust === 'signed';
}
