import type { IContent } from 'matrix-js-sdk';

import type {
  HumanMatrixPublicationGateway,
  MatrixPublicationFailure,
  MatrixPublicationRequest,
} from '@/features/messages/domain/publication';
import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';
import { err, ok } from '@/shared/result';

export class MatrixSdkHumanMessageGateway implements HumanMatrixPublicationGateway {
  readonly #clients: MatrixClientSource;

  constructor(clients: MatrixClientSource) {
    this.#clients = clients;
  }

  currentUserId(): string | null {
    return this.#clients.current()?.getUserId() ?? null;
  }

  findByTransaction(roomId: string, transactionId: string): string | null {
    const client = this.#clients.current();
    const currentUserId = client?.getUserId();
    if (client === null || currentUserId === undefined) {
      return null;
    }
    const event = client
      .getRoom(roomId)
      ?.getLiveTimeline()
      .getEvents()
      .find(
        (candidate) =>
          candidate.getTxnId() === transactionId &&
          candidate.getSender() === currentUserId &&
          candidate.getType() === 'io.github.rainyflash.agentroom.message.preview.v2',
      );
    return event?.getId() ?? null;
  }

  async publish(request: MatrixPublicationRequest) {
    const client = this.#clients.current();
    if (client?.getRoom(request.roomId) === null || client === null) {
      return err(unavailable());
    }
    try {
      const sendEvent: MatrixEventSender = client.sendEvent.bind(client);
      const response = await sendEvent(
        request.roomId,
        request.event.eventType,
        { ...request.event },
        request.transactionId,
      );
      return validMatrixEventId(response.event_id)
        ? ok(Object.freeze({ matrixEventId: response.event_id }))
        : err(rejected(false));
    } catch (error: unknown) {
      const status = httpStatus(error);
      return status !== null && status >= 400 && status < 500
        ? err(rejected(status === 408 || status === 429))
        : err(ambiguous());
    }
  }
}

type MatrixEventSender = (
  roomId: string,
  eventType: string,
  content: IContent,
  transactionId: string,
) => Promise<{ readonly event_id: string }>;

function httpStatus(error: unknown): number | null {
  if (typeof error !== 'object' || error === null) {
    return null;
  }
  const value: unknown = Reflect.get(error, 'httpStatus');
  return typeof value === 'number' && Number.isInteger(value) ? value : null;
}

function validMatrixEventId(value: string): boolean {
  return value.startsWith('$') && value.length <= 1_024;
}

function unavailable(): MatrixPublicationFailure {
  return Object.freeze({ kind: 'unavailable', retryable: true });
}

function rejected(retryable: boolean): MatrixPublicationFailure {
  return Object.freeze({ kind: 'rejected', retryable });
}

function ambiguous(): MatrixPublicationFailure {
  return Object.freeze({ kind: 'ambiguous', retryable: true });
}
