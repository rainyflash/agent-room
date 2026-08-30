import type { MatrixClient, Room } from 'matrix-js-sdk';

import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';
import { err, ok, type Result } from '@/shared/result';

import type {
  PublicLobbyEntryFailure,
  PublicLobbyMatrixGateway,
} from '@/features/lobby-entry/domain/public-lobby-entry';

const DEFAULT_JOIN_CONFIRMATION_TIMEOUT_MILLISECONDS = 15_000;

export type MatrixSdkPublicLobbyEntryGatewayOptions = {
  readonly joinConfirmationTimeoutMilliseconds?: number;
};

export class MatrixSdkPublicLobbyEntryGateway implements PublicLobbyMatrixGateway {
  readonly #joinConfirmationTimeoutMilliseconds: number;

  constructor(
    private readonly clients: MatrixClientSource,
    options: MatrixSdkPublicLobbyEntryGatewayOptions = {},
  ) {
    const timeout =
      options.joinConfirmationTimeoutMilliseconds ?? DEFAULT_JOIN_CONFIRMATION_TIMEOUT_MILLISECONDS;
    if (!Number.isFinite(timeout) || timeout <= 0) {
      throw new TypeError('Matrix 入场确认超时必须是正数。');
    }
    this.#joinConfirmationTimeoutMilliseconds = timeout;
  }

  async join(matrixRoomId: string): Promise<Result<void, PublicLobbyEntryFailure>> {
    const client = this.clients.current();
    if (client === null) {
      return err({ code: 'lobby_entry.matrix_unavailable', retryable: true });
    }
    try {
      const current = client.getRoom(matrixRoomId);
      if (current?.getMyMembership() === 'join') return ok(undefined);
      const joined = await client.joinRoom(matrixRoomId);
      const confirmed = await waitForJoinedMembership(
        this.clients,
        client,
        matrixRoomId,
        joined,
        this.#joinConfirmationTimeoutMilliseconds,
      );
      return confirmed
        ? ok(undefined)
        : err({ code: 'lobby_entry.matrix_join_unconfirmed', retryable: true });
    } catch {
      return err({ code: 'lobby_entry.matrix_join_failed', retryable: true });
    }
  }
}

async function waitForJoinedMembership(
  clients: MatrixClientSource,
  client: MatrixClient,
  matrixRoomId: string,
  joinResponseRoom: Room,
  timeoutMilliseconds: number,
): Promise<boolean> {
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
