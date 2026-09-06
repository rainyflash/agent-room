import type { MatrixClient } from 'matrix-js-sdk';

export type EncryptionReadiness = 'ready' | 'encryption_not_ready' | 'peer_verification_required';

/** 与 Bridge 的可信设备策略对齐，防止只有发送方能够解密的消息被提交。 */
export async function encryptionReadiness(
  client: MatrixClient | null,
  roomId: string,
): Promise<EncryptionReadiness> {
  const crypto = client?.getCrypto();
  const userId = client?.getUserId();
  const deviceId = client?.getDeviceId();
  if (
    crypto === undefined ||
    !userId ||
    !deviceId ||
    !(await crypto.isCrossSigningReady()) ||
    (await crypto.getDeviceVerificationStatus(userId, deviceId))?.crossSigningVerified !== true
  ) {
    return 'encryption_not_ready';
  }
  const room = client?.getRoom(roomId);
  if (!room) throw new Error('加密房间不可用');
  const members = await room.getEncryptionTargetMembers();
  const peers = [
    ...new Set(
      members
        .filter((member) => member.membership === 'join' && member.userId !== userId)
        .map((member) => member.userId),
    ),
  ];
  const devices = await crypto.getUserDeviceInfo(peers, true);
  for (const peer of peers) {
    let verified = false;
    for (const device of devices.get(peer)?.values() ?? []) {
      const trust = await crypto.getDeviceVerificationStatus(peer, device.deviceId);
      if (trust?.crossSigningVerified === true && trust.signedByOwner) {
        verified = true;
        break;
      }
    }
    if (!verified) return 'peer_verification_required';
  }
  return 'ready';
}
