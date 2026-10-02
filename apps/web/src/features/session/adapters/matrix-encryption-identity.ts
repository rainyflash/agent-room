import type { MatrixClient } from 'matrix-js-sdk';

/**
 * 没给服务器保管钥匙（测试、旧的组装）时的退路：从未建立过加密身份的账户（第一台设备）
 * 直接建立。正式的做法是 `matrix-device-signing.ts` 的自动签名（ADR 0011）。
 *
 * 加密房间只把房间密钥发给由主人签名的设备，没有身份就收不到别人的加密消息，也发不出去。
 * 已有身份时绝不重建。失败不影响登录。
 */
export async function ensureFirstEncryptionIdentity(
  client: Pick<MatrixClient, 'getCrypto' | 'getUserId'>,
): Promise<void> {
  try {
    const crypto = client.getCrypto();
    const userId = client.getUserId();
    if (crypto === undefined || !userId) return;
    if (await crypto.userHasCrossSigningKeys(userId, true)) return;
    await crypto.bootstrapCrossSigning({
      authUploadDeviceSigningKeys: async (makeRequest) => {
        await makeRequest(null);
      },
    });
  } catch {
    // 交给安全中心：身份查询或首次上传失败时，用户仍可在那里建立或恢复。
  }
}
