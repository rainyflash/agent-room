import { encryptContent } from './browser-content-cipher';
import type {
  MessagePublicationRequest,
  PreparedMessageBody,
  MessagePublicationFailure,
  ProtectedMessageBody,
} from '../domain/publication';
import type { Result } from '@/shared/result';
import { EventStatus, type IContent, type MatrixClient } from 'matrix-js-sdk';

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

  async #encrypted(roomId: string): Promise<boolean> {
    const client = this.#clients.current();
    if (client === null || client.getRoom(roomId)?.getMyMembership() !== 'join')
      throw new Error('当前会话不可用');
    try {
      const state: unknown = await client.getStateEvent(roomId, 'm.room.encryption', '');
      if (
        typeof state !== 'object' ||
        state === null ||
        Reflect.get(state, 'algorithm') !== 'm.megolm.v1.aes-sha2'
      )
        throw new Error('不支持的房间加密算法');
      return true;
    } catch (error: unknown) {
      if (
        httpStatus(error) === 404 &&
        typeof error === 'object' &&
        error !== null &&
        Reflect.get(error, 'errcode') === 'M_NOT_FOUND'
      )
        return false;
      throw error;
    }
  }

  async protectBody(
    request: MessagePublicationRequest,
    body: PreparedMessageBody,
  ): Promise<Result<ProtectedMessageBody, MessagePublicationFailure>> {
    try {
      if (!(await this.#encrypted(request.roomId))) return ok({ body });
      if (!(await encryptionIdentityReady(this.#clients.current())))
        return err({ code: 'publication.encryption_not_ready', retryable: true });
      return ok(
        await encryptContent(body, request.submissionId, request.roomId, request.mediaType),
      );
    } catch {
      return err({ code: 'publication.matrix_rejected', retryable: true });
    }
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
          (candidate.status === null || candidate.status === EventStatus.SENT) &&
          validMatrixEventId(candidate.getId() ?? '') &&
          candidate.getType() === 'io.github.rainyflash.agentroom.message.preview.v2',
      );
    return event?.getId() ?? null;
  }

  async publish(
    request: MatrixPublicationRequest,
  ): ReturnType<HumanMatrixPublicationGateway['publish']> {
    const client = this.#clients.current();
    if (client?.getRoom(request.roomId) === null || client === null) {
      return err(unavailable());
    }
    let encrypted = false;
    let submissionStarted = false;
    try {
      if (request.event.actor.matrixUserId !== client.getUserId()) return err(rejected(false));
      encrypted = await this.#encrypted(request.roomId);
      if (encrypted !== (request.event.content.encryption !== undefined))
        return err(rejected(false));
      if (encrypted && !(await client.getCrypto()?.isEncryptionEnabledInRoom(request.roomId)))
        return err(unavailable());
      if (encrypted && !(await encryptionIdentityReady(client)))
        return err({ kind: 'encryption_not_ready', retryable: true });
      const sendEvent: MatrixEventSender = client.sendEvent.bind(client);
      submissionStarted = true;
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
      const local = client
        .getRoom(request.roomId)
        ?.getLiveTimeline()
        .getEvents()
        .find(
          (event) =>
            event.getTxnId() === request.transactionId && event.getSender() === client.getUserId(),
        );
      // SDK 在加密成功后才发送 HTTP。未加密的失败本地回显不代表未知网络提交，
      // 必须清除其待发送占位，避免后续重试被同一事务或失败队列阻塞。
      if (encrypted && local?.status === EventStatus.NOT_SENT && !local.isEncrypted()) {
        client.cancelPendingEvent(local);
        return err({ kind: 'encryption_not_ready', retryable: true });
      }
      const status = httpStatus(error);
      return status !== null && status >= 400 && status < 500
        ? err(rejected(status === 408 || status === 429))
        : err(submissionStarted ? ambiguous() : unavailable());
    }
  }
}

async function encryptionIdentityReady(client: MatrixClient | null): Promise<boolean> {
  const crypto = client?.getCrypto();
  const userId = client?.getUserId();
  const deviceId = client?.getDeviceId();
  if (crypto === undefined || !userId || !deviceId || !(await crypto.isCrossSigningReady()))
    return false;
  return (
    (await crypto.getDeviceVerificationStatus(userId, deviceId))?.crossSigningVerified === true
  );
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
