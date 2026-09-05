import type {
  HumanMatrixPublicationGateway,
  HumanMessagePreviewEvent,
  MessageBodyPreparer,
  MessagePublicationContentGateway,
  MessagePublicationFailure,
  MessagePublicationRequest,
  MessagePublicationResult,
  MessagePublisher,
  MessagePublisherIdentity,
  MessageSubmissionJournal,
  MessageSubmissionRecord,
  PublicationProgressStage,
  PreparedMessageBody,
} from '@/features/messages/domain/publication';
import { validatePublicationRequest } from '@/features/messages/domain/publication';
import type { ControlPlaneGateway } from '@/features/session/domain/session';
import { err, ok, type Result } from '@/shared/result';

const eventType = 'io.github.rainyflash.agentroom.message.preview.v2' as const;
const transactionPrefix = 'agent-room-message-';
const uuidV7Pattern = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u;

export type HumanMessagePublisherDependencies = {
  readonly bodyPreparer: MessageBodyPreparer;
  readonly clock?: () => number;
  readonly content: MessagePublicationContentGateway;
  readonly journal: MessageSubmissionJournal;
  readonly matrix: HumanMatrixPublicationGateway;
  readonly session: Pick<ControlPlaneGateway, 'readSession'>;
};

export class HumanMessagePublisher implements MessagePublisher {
  readonly #bodyPreparer: MessageBodyPreparer;
  readonly #clock: () => number;
  readonly #content: MessagePublicationContentGateway;
  readonly #journal: MessageSubmissionJournal;
  readonly #matrix: HumanMatrixPublicationGateway;
  readonly #session: Pick<ControlPlaneGateway, 'readSession'>;

  constructor({
    bodyPreparer,
    clock = Date.now,
    content,
    journal,
    matrix,
    session,
  }: HumanMessagePublisherDependencies) {
    this.#bodyPreparer = bodyPreparer;
    this.#clock = clock;
    this.#content = content;
    this.#journal = journal;
    this.#matrix = matrix;
    this.#session = session;
  }

  async resolveIdentity(): Promise<Result<MessagePublisherIdentity, MessagePublicationFailure>> {
    const session = await this.#session.readSession();
    if (!session.ok) {
      return err(identityFailure(session.error.retryable, session.error.correlationId));
    }
    const matrixUserId = this.#matrix.currentUserId();
    if (
      matrixUserId === null ||
      matrixUserId !== session.value.matrixUserId ||
      !uuidV7Pattern.test(session.value.principalId) ||
      !validActorDisplayName(session.value.displayName)
    ) {
      return err(identityFailure(true));
    }
    return ok(
      Object.freeze({
        displayName: session.value.displayName,
        kind: 'human',
        matrixUserId,
        principalId: session.value.principalId,
        source: 'matrix_human_session',
      }),
    );
  }

  async publish(
    request: MessagePublicationRequest,
    onProgress: (stage: PublicationProgressStage) => void,
  ): Promise<MessagePublicationResult> {
    if (validatePublicationRequest(request).length > 0) {
      return err(Object.freeze({ code: 'publication.invalid_intent', retryable: false }));
    }
    const identity = await this.resolveIdentity();
    if (!identity.ok) {
      return identity;
    }
    const prepared = await this.#bodyPreparer.prepare(request.body);
    if (!prepared.ok) {
      return prepared;
    }
    const fingerprint = await this.#fingerprint(request, prepared.value);
    if (!fingerprint.ok) {
      return fingerprint;
    }
    const existing = this.#journal.read(request.submissionId);
    if (!existing.ok) {
      return existing;
    }
    if (existing.value !== null) {
      if (existing.value.fingerprint !== fingerprint.value) {
        return err(Object.freeze({ code: 'publication.invalid_intent', retryable: false }));
      }
      return await this.#submitRecord(existing.value, true, onProgress);
    }

    // 首次上传前持久化密文，网络重试和重新加载必须复用同一密钥、随机数与幂等键。
    const scope = `${identity.value.matrixUserId}:${identity.value.principalId}:${fingerprint.value}`;
    const cached = this.#journal.readBody(scope);
    if (!cached.ok) return cached;
    const protectedBody =
      cached.value === null
        ? await this.#matrix.protectBody(request, prepared.value)
        : ok(cached.value);
    if (!protectedBody.ok) return protectedBody;
    const cachedWrite = this.#journal.writeBody(scope, protectedBody.value);
    if (!cachedWrite.ok) return cachedWrite;
    onProgress('uploading');
    const uploaded = await this.#content.upload({
      body: protectedBody.value.body,
      encryptionMode: protectedBody.value.encryption === undefined ? 'server_side' : 'client_e2ee',
      mediaType: request.mediaType,
      roomId: request.roomId,
      submissionId: request.submissionId,
    });
    if (!uploaded.ok) {
      return uploaded;
    }
    const record = createRecord(
      request,
      identity.value,
      {
        ...uploaded.value,
        ...(protectedBody.value.encryption === undefined
          ? {}
          : { encryption: protectedBody.value.encryption }),
      },
      fingerprint.value,
      this.#clock(),
    );
    const written = this.#journal.write(record);
    if (!written.ok) {
      return written;
    }
    this.#journal.releaseBody(scope, request.submissionId);
    return await this.#submitRecord(record, false, onProgress);
  }

  async reconcile(submissionId: string): Promise<MessagePublicationResult> {
    const stored = this.#journal.read(submissionId);
    if (!stored.ok) {
      return stored;
    }
    if (stored.value === null) {
      return err(Object.freeze({ code: 'publication.persistence_failed', retryable: false }));
    }
    return await this.#submitRecord(stored.value, true, noopProgress);
  }

  async #fingerprint(
    request: MessagePublicationRequest,
    prepared: PreparedMessageBody,
  ): Promise<Result<string, MessagePublicationFailure>> {
    const canonical = JSON.stringify({
      bodyDigestSha256: prepared.digestSha256,
      conversation: request.conversation ?? null,
      relation: request.relation ?? null,
      language: request.language ?? null,
      mediaType: request.mediaType,
      riskFlags: [...request.riskFlags],
      roomId: request.roomId,
      sensitivity: request.sensitivity,
      submissionId: request.submissionId,
      summary: request.summary,
      title: request.title,
    });
    const fingerprint = await this.#bodyPreparer.prepare(canonical);
    return fingerprint.ok ? ok(fingerprint.value.digestSha256) : fingerprint;
  }

  async #submitRecord(
    record: MessageSubmissionRecord,
    reused: boolean,
    onProgress: (stage: PublicationProgressStage) => void,
  ): Promise<MessagePublicationResult> {
    const identity = await this.resolveIdentity();
    if (!identity.ok) return identity;
    if (
      record.event.actor.matrixUserId !== identity.value.matrixUserId ||
      record.event.actor.principalId !== identity.value.principalId
    )
      return err(identityFailure(false));
    let matrixEventId = record.matrixEventId;
    matrixEventId ??=
      this.#matrix.findByTransaction(record.roomId, record.transactionId) ?? undefined;
    if (matrixEventId === undefined) {
      onProgress('submitting');
      const published = await this.#matrix.publish({
        event: record.event,
        roomId: record.roomId,
        transactionId: record.transactionId,
      });
      if (!published.ok) {
        if (published.error.kind === 'ambiguous') {
          return ok(pending(record));
        }
        return err(
          Object.freeze({
            code: 'publication.matrix_rejected',
            retryable: published.error.retryable,
          }),
        );
      }
      matrixEventId = published.value.matrixEventId;
    }

    const acceptedRecord = Object.freeze({ ...record, matrixEventId });
    const written = this.#journal.write(acceptedRecord);
    if (!written.ok) {
      return ok(pending(record));
    }
    onProgress('binding');
    const bound = await this.#content.bind({
      contentId: record.content.contentId,
      matrixEventId,
      roomId: record.roomId,
    });
    if (!bound.ok) {
      return ok(
        Object.freeze({
          kind: 'accepted_binding_pending',
          matrixEventId,
          submissionId: record.submissionId,
        }),
      );
    }
    return ok(
      Object.freeze({
        kind: 'published',
        matrixEventId,
        reused,
        submissionId: record.submissionId,
      }),
    );
  }
}

function createRecord(
  request: MessagePublicationRequest,
  identity: MessagePublisherIdentity,
  content: MessageSubmissionRecord['content'],
  fingerprint: string,
  createdAtUnixMs: number,
): MessageSubmissionRecord {
  const actor = Object.freeze({
    displayName: identity.displayName,
    kind: identity.kind,
    matrixUserId: identity.matrixUserId,
    principalId: identity.principalId,
  });
  const eventContent = Object.freeze({ ...content, fetchMode: 'on_demand' as const });
  const preview = Object.freeze({
    ...(request.conversation === undefined ? {} : { conversation: request.conversation }),
    contentType: request.mediaType,
    ...(request.language === undefined ? {} : { language: request.language }),
    riskFlags: Object.freeze([...request.riskFlags]),
    sensitivity: request.sensitivity,
    summary: request.summary,
    title: request.title,
  });
  const event: HumanMessagePreviewEvent = Object.freeze({
    actor,
    content: eventContent,
    correlationId: request.submissionId,
    createdAt: new Date(createdAtUnixMs).toISOString(),
    eventType,
    id: request.submissionId,
    preview,
    ...(request.relation === undefined ? {} : { relation: request.relation }),
    roomId: request.roomId,
    schemaVersion: '2.0',
  });
  return Object.freeze({
    content,
    event,
    fingerprint,
    roomId: request.roomId,
    submissionId: request.submissionId,
    transactionId: `${transactionPrefix}${request.submissionId}`,
  });
}

function pending(record: MessageSubmissionRecord) {
  return Object.freeze({
    kind: 'pending_reconciliation' as const,
    submissionId: record.submissionId,
    transactionId: record.transactionId,
  });
}

function identityFailure(retryable: boolean, correlationId?: string): MessagePublicationFailure {
  return Object.freeze({
    code: 'publication.identity_unavailable',
    ...(correlationId === undefined ? {} : { correlationId }),
    retryable,
  });
}

function noopProgress(): void {
  return undefined;
}

function validActorDisplayName(value: string): boolean {
  let characters = 0;
  for (const character of value) {
    characters += 1;
    const codePoint = character.codePointAt(0) ?? 0;
    if (codePoint <= 31 || codePoint === 127) {
      return false;
    }
  }
  return value.trim().length > 0 && characters <= 80;
}
