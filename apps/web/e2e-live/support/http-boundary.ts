export const apiOrigin = 'https://api.agent-room.localhost:18443';
export const matrixOrigin = 'https://matrix.agent-room.localhost:18443';

const optionalAccountData = new Set([
  'io.github.rainyflash.agentroom.preferences.v1',
  'io.github.rainyflash.agentroom.personal-workspace.v1',
  'm.secret_storage.default_key',
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
    (status === 404 &&
      method === 'GET' &&
      url.origin === matrixOrigin &&
      (url.pathname === '/_matrix/client/unstable/org.matrix.msc4143/rtc/transports' ||
        url.pathname === '/_matrix/client/v3/room_keys/version' ||
        missingInitialAccountData))
  );
}
