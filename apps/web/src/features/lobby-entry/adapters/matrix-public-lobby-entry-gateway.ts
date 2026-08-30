import { RoomEvent, type Room } from 'matrix-js-sdk';

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
        client.getRoom(matrixRoomId) ?? joined,
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

async function waitForJoinedMembership(room: Room, timeoutMilliseconds: number): Promise<boolean> {
  if (room.getMyMembership() === 'join') return true;
  return await new Promise<boolean>((resolve) => {
    let timeout: ReturnType<typeof globalThis.setTimeout> | null = null;
    let settled = false;
    const finish = (joined: boolean): void => {
      if (settled) return;
      settled = true;
      room.off(RoomEvent.MyMembership, onMembership);
      if (timeout !== null) globalThis.clearTimeout(timeout);
      resolve(joined);
    };
    const onMembership = (_room: Room, membership: ReturnType<Room['getMyMembership']>): void => {
      if (membership === 'join') finish(true);
    };
    room.on(RoomEvent.MyMembership, onMembership);
    if (room.getMyMembership() === 'join') {
      finish(true);
      return;
    }
    timeout = globalThis.setTimeout(() => finish(false), timeoutMilliseconds);
  });
}
