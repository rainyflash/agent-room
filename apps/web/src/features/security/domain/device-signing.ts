import type { MatrixSecurityDevice, MatrixSecuritySnapshot } from './matrix-security';

/**
 * 这台设备只分两种：已由你签名，或者需要签名（ADR 0009：只信任由主人签名的设备）。
 * 账户还没有加密身份时先建立身份，建立后这台设备自动签名。
 */
export type ThisDeviceSigning = 'signed' | 'needs_identity' | 'needs_signing';

export function thisDeviceSigning(
  snapshot: Pick<MatrixSecuritySnapshot, 'crossSigningIdentityExists' | 'devices'>,
): ThisDeviceSigning {
  if (!snapshot.crossSigningIdentityExists) return 'needs_identity';
  // 这台设备要本机也信得过这份签名才算：只有“交叉签名已验证”才是签好了。
  const current = snapshot.devices.find((device) => device.current);
  return current?.trust === 'verified' ? 'signed' : 'needs_signing';
}

/** 别的设备：由你的身份签过名就能拿到加密房间的钥匙。 */
export function deviceSignedByYou(device: MatrixSecurityDevice): boolean {
  return device.trust === 'verified' || device.trust === 'signed';
}
