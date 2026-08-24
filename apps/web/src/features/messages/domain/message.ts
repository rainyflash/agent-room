import type { Result } from '@/shared/result';

export const messageProvenances = ['human', 'human_confirmed_agent', 'autonomous_agent'] as const;
export const messageSensitivities = ['normal', 'sensitive', 'restricted'] as const;
export const messageSignatureStatuses = [
  'instance_verified',
  'matrix_sender_matched',
  'revoked_after_event',
] as const;

export type MessageProvenance = (typeof messageProvenances)[number];
export type MessageSensitivity = (typeof messageSensitivities)[number];
export type MessageSignatureStatus = (typeof messageSignatureStatuses)[number];
export type MessageLifecycle = 'active' | 'moderated' | 'redacted';

export type MessageActor = {
  readonly agentId: string;
  readonly avatarUrl?: string;
  readonly displayName: string;
  readonly instanceId: string;
  readonly matrixUserId: string;
  readonly provenance: MessageProvenance;
};

export type MessageContentReference = {
  readonly contentId: string;
  readonly digestSha256: string;
  readonly mediaType: string;
  readonly sizeBytes: number;
};

export type MessagePreview = {
  readonly contentType: string;
  readonly language?: string;
  readonly riskFlags: readonly string[];
  readonly sensitivity: MessageSensitivity;
  readonly summary: string;
  readonly title: string;
};

export type MessageRelation = {
  readonly kind: 'reply';
  readonly targetMessageId: string;
};

export type RoomMessageSignal = {
  readonly actor: MessageActor;
  readonly content: MessageContentReference | null;
  readonly edited: boolean;
  readonly lifecycle: MessageLifecycle;
  readonly matrixEventId: string;
  readonly messageId: string;
  readonly preview: MessagePreview | null;
  readonly relation?: MessageRelation;
  readonly roomId: string;
  readonly serverTimestamp: number;
  readonly signatureStatus: MessageSignatureStatus;
};

export type MessageRoomProjection = {
  readonly messages: readonly RoomMessageSignal[];
  readonly observedAtUnixMs: number;
  readonly roomId: string;
};

export type MessageFailureCode =
  'messages.matrix_unavailable' | 'messages.room_not_joined' | 'messages.projection_invalid';

export type MessageFailure = {
  readonly code: MessageFailureCode;
  readonly retryable: boolean;
};

export type MessageReadResult = Result<MessageRoomProjection, MessageFailure>;

export type MessageGateway = {
  read(roomId: string): MessageReadResult;
  subscribe(roomId: string, listener: () => void): () => void;
};
