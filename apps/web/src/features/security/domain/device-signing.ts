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
  // 由你的签名身份签过就算签好了：Agent 给不给房间密钥只看这个，找回历史也只看这个。
  // 本机有没有签名私钥（能不能再去签别的设备）不影响这台设备收发加密消息；以前把它也算进来，
  // 签过名的桌面端每次打开都说“需要签名”（2026-10-02 维护者遇到过）。
  const current = snapshot.devices.find((device) => device.current);
  return current !== undefined && deviceSignedByYou(current) ? 'signed' : 'needs_signing';
}

/** 别的设备：由你的身份签过名就能拿到加密房间的钥匙。 */
export function deviceSignedByYou(device: MatrixSecurityDevice): boolean {
  return device.trust === 'verified' || device.trust === 'signed';
}
