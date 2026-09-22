import type { MatrixClient } from 'matrix-js-sdk';

/**
 * 从未建立过加密身份的账户（第一台设备）直接建立，不必去安全中心手动操作。
 *
 * 加密房间只把房间密钥发给由主人签名的设备，没有身份就收不到别人的加密消息，也发不出去。
 * 已有身份时绝不重建：新设备要在已登录的设备上确认，或用恢复密钥恢复。失败不影响登录，
 * 安全中心仍可手动建立。
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
