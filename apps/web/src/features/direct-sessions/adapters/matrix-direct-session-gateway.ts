import {
  EventType,
  ReceiptType,
  type MatrixClient,
  type Room,
} from 'matrix-js-sdk';
import { z } from 'zod';

import type {
  DirectSession,
  DirectSessionFailure,
  DirectSessionMatrixGateway,
} from '@/features/direct-sessions/domain/direct-session';
import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';
import { err, ok, type Result } from '@/shared/result';

const directAccountDataSchema = z.record(
  z
    .string()
    .min(4)
    .max(255)
    .regex(/^@[^:]+:[^:]+$/u),
  z.array(
    z
      .string()
      .min(4)
      .max(255)
      .regex(/^![^:]+:[^:]+$/u),
  ),
);

export class MatrixSdkDirectSessionGateway implements DirectSessionMatrixGateway {
  readonly #clients: MatrixClientSource;

  constructor(clients: MatrixClientSource) {
    this.#clients = clients;
  }

  async prepare(session: DirectSession): Promise<Result<void, DirectSessionFailure>> {
    const client = this.#clients.current();
    if (client === null || session.matrixRoomId === null) {
      return err(failure('direct_session.matrix_unavailable'));
    }
    try {
      const room = await ensureJoined(client, session.matrixRoomId);
      if (room.getMyMembership() !== 'join') {
        return err(failure('direct_session.join_failed'));
      }
      const direct = await client.getAccountDataFromServer(EventType.Direct);
      const parsed = directAccountDataSchema.safeParse(direct ?? {});
      if (!parsed.success) {
        return err({ code: 'direct_session.invalid_matrix_account_data', retryable: false });
      }
      const rooms = parsed.data[session.target.matrixUserId] ?? [];
      if (!rooms.includes(session.matrixRoomId)) {
        await client.setAccountData(EventType.Direct, {
          ...parsed.data,
          [session.target.matrixUserId]: [...rooms, session.matrixRoomId],
        });
      }
      return ok(undefined);
    } catch {
      return err(failure('direct_session.matrix_prepare_failed'));
    }
  }

  async setIgnored(
    matrixUserId: string,
    ignored: boolean,
  ): Promise<Result<void, DirectSessionFailure>> {
    const client = this.#clients.current();
    if (client === null) {
      return err(failure('direct_session.matrix_unavailable'));
    }
    try {
      const current = client.getIgnoredUsers();
      const exists = current.includes(matrixUserId);
      if (exists !== ignored) {
        await client.setIgnoredUsers(
          ignored
            ? [...current, matrixUserId]
            : current.filter((candidate) => candidate !== matrixUserId),
        );
      }
      return ok(undefined);
    } catch {
      return err(failure('direct_session.ignore_sync_failed'));
    }
  }

  async markDisplayed(
    roomId: string,
    matrixEventId: string,
  ): Promise<Result<void, DirectSessionFailure>> {
    const client = this.#clients.current();
    if (client === null) {
      return err(failure('direct_session.matrix_unavailable'));
    }
    const event = client.getRoom(roomId)?.findEventById(matrixEventId);
    if (event === undefined) {
      return err({ code: 'direct_session.receipt_event_missing', retryable: false });
    }
    try {
      await client.sendReadReceipt(event, ReceiptType.ReadPrivate, true);
      return ok(undefined);
    } catch {
      return err(failure('direct_session.receipt_failed'));
    }
  }
}

async function ensureJoined(client: MatrixClient, roomId: string): Promise<Room> {
  const current = client.getRoom(roomId);
  return current?.getMyMembership() === 'join' ? current : await client.joinRoom(roomId);
}

function failure(code: string): DirectSessionFailure {
  return { code, retryable: true };
}
