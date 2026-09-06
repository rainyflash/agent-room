import { joinConfirmedMatrixRoom } from '@/shared/matrix/join-confirmed-matrix-room';

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
      const confirmed = await joinConfirmedMatrixRoom(
        this.clients,
        client,
        matrixRoomId,
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
