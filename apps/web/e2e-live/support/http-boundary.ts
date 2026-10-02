export const apiOrigin = 'https://api.agent-room.localhost:18443';
export const matrixOrigin = 'https://matrix.agent-room.localhost:18443';

const optionalAccountData = new Set([
  'io.github.rainyflash.agentroom.preferences.v1',
  'io.github.rainyflash.agentroom.personal-workspace.v1',
  'm.secret_storage.default_key',
  'm.megolm_backup.v1',
  'm.cross_signing.master',
  'm.cross_signing.self_signing',
  'm.cross_signing.user_signing',
]);

export function isExpectedHttpBoundary(status: number, rawUrl: string, method: string): boolean {
  const url = new URL(rawUrl);
  const accountDataType = /^\/_matrix\/client\/v3\/user\/[^/]+\/account_data\/([^/]+)$/u.exec(
    url.pathname,
  )?.[1];
  // Matrix specifies 404 for unset account data. Fresh accounts have no workspace or keys yet.
  const missingInitialAccountData =
    accountDataType !== undefined && optionalAccountData.has(accountDataType);
  return (
    (status === 401 && url.origin === apiOrigin && url.pathname === '/auth/session') ||
    // 全新账户的第一台设备：服务器上还没有替账户保管的签名钥匙（ADR 0011），设备随后新建一把。
    (status === 404 &&
      method === 'GET' &&
      url.origin === apiOrigin &&
      url.pathname === '/account/encryption-key') ||
    // 重建签名身份时 matrix-js-sdk 顺手删掉“脱水设备”；账户没有时 Synapse 回 404，SDK 也当作正常。
    (status === 404 &&
      (method === 'GET' || method === 'DELETE') &&
      url.origin === matrixOrigin &&
      url.pathname === '/_matrix/client/unstable/org.matrix.msc3814.v1/dehydrated_device') ||
    (status === 404 &&
      method === 'GET' &&
      url.origin === matrixOrigin &&
      (url.pathname === '/_matrix/client/unstable/org.matrix.msc4143/rtc/transports' ||
        url.pathname === '/_matrix/client/v3/room_keys/version' ||
        missingInitialAccountData))
  );
}
