import type { MatrixClient, Room } from 'matrix-js-sdk';
import type { MatrixClientSource } from './matrix-client-registry';

/** 加入接口确认请求被接收；只有同步投影确认成员身份后才能开放房间操作。 */
export async function joinConfirmedMatrixRoom(
  clients: MatrixClientSource,
  client: MatrixClient,
  matrixRoomId: string,
  timeoutMilliseconds = 15_000,
): Promise<boolean> {
  if (clients.current() !== client) return false;
  if (client.getRoom(matrixRoomId)?.getMyMembership() === 'join') return true;
  const joined = await client.joinRoom(matrixRoomId);
  return await waitForJoinedMembership(clients, client, matrixRoomId, joined, timeoutMilliseconds);
}

async function waitForJoinedMembership(
  clients: MatrixClientSource,
  client: MatrixClient,
  matrixRoomId: string,
  joinResponseRoom: Room,
  timeoutMilliseconds: number,
): Promise<boolean> {
  if (clients.current() !== client) return false;
  if (hasJoinedMembership(client, matrixRoomId, joinResponseRoom)) return true;
  return await new Promise<boolean>((resolve) => {
    let timeout: ReturnType<typeof globalThis.setTimeout> | null = null;
    let unsubscribe: (() => void) | null = null;
    let settled = false;
    const finish = (joined: boolean): void => {
      if (settled) return;
      settled = true;
      unsubscribe?.();
      if (timeout !== null) globalThis.clearTimeout(timeout);
      resolve(joined);
    };
    const inspectProjection = (): boolean => {
      if (clients.current() !== client) {
        finish(false);
        return true;
      }
      if (hasJoinedMembership(client, matrixRoomId, joinResponseRoom)) {
        finish(true);
        return true;
      }
      return false;
    };
    unsubscribe = clients.subscribe(() => {
      inspectProjection();
    });
    if (inspectProjection()) {
      unsubscribe();
      return;
    }
    timeout = globalThis.setTimeout(() => {
      finish(false);
    }, timeoutMilliseconds);
  });
}

function hasJoinedMembership(
  client: MatrixClient,
  matrixRoomId: string,
  joinResponseRoom: Room,
): boolean {
  return (
    client.getRoom(matrixRoomId)?.getMyMembership() === 'join' ||
    joinResponseRoom.getMyMembership() === 'join'
  );
}
