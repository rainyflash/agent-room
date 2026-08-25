import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';
import { err, ok, type Result } from '@/shared/result';

import type {
  PrivateRoomFailure,
  PrivateRoomMatrixGateway,
} from '@/features/private-rooms/domain/private-room';

export class MatrixSdkPrivateRoomGateway implements PrivateRoomMatrixGateway {
  readonly #clients: MatrixClientSource;

  constructor(clients: MatrixClientSource) {
    this.#clients = clients;
  }

  async join(roomId: string): Promise<Result<void, PrivateRoomFailure>> {
    const client = this.#clients.current();
    if (client === null) {
      return err({ code: 'private_room.matrix_unavailable', retryable: true });
    }
    try {
      await client.joinRoom(roomId);
      return ok(undefined);
    } catch {
      return err({ code: 'private_room.matrix_join_failed', retryable: true });
    }
  }

  async leave(roomId: string): Promise<Result<void, PrivateRoomFailure>> {
    const client = this.#clients.current();
    if (client === null) {
      return err({ code: 'private_room.matrix_unavailable', retryable: true });
    }
    try {
      await client.leave(roomId);
      return ok(undefined);
    } catch {
      return err({ code: 'private_room.matrix_leave_failed', retryable: true });
    }
  }
}
