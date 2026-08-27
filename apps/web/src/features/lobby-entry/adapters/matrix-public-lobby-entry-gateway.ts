import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';
import { err, ok, type Result } from '@/shared/result';

import type {
  PublicLobbyEntryFailure,
  PublicLobbyMatrixGateway,
} from '@/features/lobby-entry/domain/public-lobby-entry';

export class MatrixSdkPublicLobbyEntryGateway implements PublicLobbyMatrixGateway {
  constructor(private readonly clients: MatrixClientSource) {}

  async join(matrixRoomId: string): Promise<Result<void, PublicLobbyEntryFailure>> {
    const client = this.clients.current();
    if (client === null) {
      return err({ code: 'lobby_entry.matrix_unavailable', retryable: true });
    }
    try {
      const current = client.getRoom(matrixRoomId);
      if (current?.getMyMembership() === 'join') return ok(undefined);
      const joined = await client.joinRoom(matrixRoomId);
      return joined.getMyMembership() === 'join'
        ? ok(undefined)
        : err({ code: 'lobby_entry.matrix_join_unconfirmed', retryable: true });
    } catch {
      return err({ code: 'lobby_entry.matrix_join_failed', retryable: true });
    }
  }
}
