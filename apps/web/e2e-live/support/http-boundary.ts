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

// 密钥备份里某个房间的某一把会话密钥；房间 ID 和会话 ID 都转义过，路径里没有别的斜杠。
const roomKeyBackupSession = /^\/_matrix\/client\/v3\/room_keys\/keys\/[^/]+\/[^/]+$/u;
// 私人房间现在的邀请链接；还没生成过时回 404 `invite_link.not_found`，“邀请人”里说还没有链接。
const privateRoomInviteLink = /^\/private-rooms\/[0-9a-f-]{36}\/invite-link$/u;

// 发话前问房间加没加密（`matrix-human-message-gateway.ts`）：不加密的公开房间、公共大厅没有这条状态，回 404。
const roomEncryptionState = /^\/_matrix\/client\/v3\/rooms\/[^/]+\/state\/m\.room\.encryption\/?$/u;

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
    // 账户的两台设备同时第一次登录：都去建签名身份，后传公钥的那台被要求交互认证。它什么都没动，
    // 随后用先到那台交给服务器的钥匙签好自己（`matrix-device-signing.ts`，登录验收有一条专门走这里）。
    (status === 401 &&
      method === 'POST' &&
      url.origin === matrixOrigin &&
      url.pathname === '/_matrix/client/v3/keys/device_signing/upload') ||
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
    // 新设备解不开以前的消息时，按会话去密钥备份里一把一把找；发送那台设备还没把这把传上去，
    // Synapse 就回 404，SDK 当作“备份里没有”。验收账号在前面几轮里收发过加密消息，换新设备登录就会碰上。
    (status === 404 &&
      method === 'GET' &&
      url.origin === matrixOrigin &&
      roomKeyBackupSession.test(url.pathname)) ||
    (status === 404 &&
      method === 'GET' &&
      url.origin === apiOrigin &&
      privateRoomInviteLink.test(url.pathname)) ||
    (status === 404 &&
      method === 'GET' &&
      url.origin === matrixOrigin &&
      roomEncryptionState.test(url.pathname)) ||
    (status === 404 &&
      method === 'GET' &&
      url.origin === matrixOrigin &&
      (url.pathname === '/_matrix/client/unstable/org.matrix.msc4143/rtc/transports' ||
        url.pathname === '/_matrix/client/v3/room_keys/version' ||
        missingInitialAccountData))
  );
}
