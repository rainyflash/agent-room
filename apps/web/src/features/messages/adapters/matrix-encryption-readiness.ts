import type { MatrixClient } from 'matrix-js-sdk';

export type EncryptionReadiness = 'ready' | 'encryption_not_ready' | 'identity_changed';

/**
 * 与 Bridge 的信任策略一致（MSC4153）：房间密钥只发给由主人签名的设备，首次见到的加密身份被记住，
 * 不要求与参与者逐个核对安全码。发送前只检查本机设备已由自己的身份签名，以及房间里有没有人换了身份。
 *
 * 有人换了身份（重装、重置加密，或有人冒充）时，第一次发送返回 `identity_changed` 并记下这些人。
 * 用户看过提示后再次发送就是确认：记住他们的新身份（曾核对过的一并撤销核对要求）后继续。
 * 重试只由用户按下「确认并发送」触发，不会自动发生。
 */
export class IdentityChangeAcknowledgements {
  readonly #warned = new Map<string, ReadonlySet<string>>();

  async readiness(client: MatrixClient | null, roomId: string): Promise<EncryptionReadiness> {
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
    const changed = new Map<string, boolean>();
    for (const peer of peers) {
      const status = await crypto.getUserVerificationStatus(peer);
      if (status.needsUserApproval) changed.set(peer, status.wasCrossSigningVerified());
    }
    const warned = this.#warned.get(roomId);
    if (changed.size === 0) {
      this.#warned.delete(roomId);
      return 'ready';
    }
    if (warned === undefined || [...changed.keys()].some((peer) => !warned.has(peer))) {
      this.#warned.set(roomId, new Set(changed.keys()));
      return 'identity_changed';
    }
    for (const [peer, wasVerified] of changed) {
      if (wasVerified) await crypto.withdrawVerificationRequirement(peer);
      await crypto.pinCurrentUserIdentity(peer);
    }
    this.#warned.delete(roomId);
    return 'ready';
  }
}
