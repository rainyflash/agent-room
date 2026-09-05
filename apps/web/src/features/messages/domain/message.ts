import type { ConversationMessage } from '@/features/conversation/domain/conversation';
import type { ClientContentEncryption } from './content-encryption';
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

export type HumanMessageActor = {
  readonly avatarUrl?: string;
  readonly displayName: string;
  readonly kind: 'human';
  readonly matrixUserId: string;
  readonly principalId: string;
  readonly provenance: 'human';
};

export type AgentMessageActor = {
  readonly agentId: string;
  readonly avatarUrl?: string;
  readonly displayName: string;
  readonly instanceId: string;
  readonly kind: 'agent';
  readonly matrixUserId: string;
  readonly provenance: Extract<MessageProvenance, 'human_confirmed_agent' | 'autonomous_agent'>;
};

export type MessageActor = HumanMessageActor | AgentMessageActor;

export type MessageContentReference = {
  readonly encryption?: ClientContentEncryption;
  readonly contentId: string;
  readonly digestSha256: string;
  readonly mediaType: string;
  readonly sizeBytes: number;
};

export type MessagePreview = {
  readonly conversation?: ConversationMessage;
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
  readonly endToEndEncrypted: boolean;
  readonly lifecycle: MessageLifecycle;
  readonly matrixEventId: string;
  readonly messageId: string;
  readonly preview: MessagePreview | null;
  readonly relation?: MessageRelation;
  readonly roomId: string;
  readonly serverTimestamp: number;
  readonly signatureStatus: MessageSignatureStatus;
};

export type ReadOnlyFederatedEventReason = 'legacy_namespace' | 'unknown_event_type';

export type ReadOnlyFederatedEvent = {
  readonly endToEndEncrypted: boolean;
  readonly eventType: string;
  readonly matrixEventId: string;
  readonly reason: ReadOnlyFederatedEventReason;
  readonly sender: string;
  readonly serverTimestamp: number;
};

export type MessageRoomProjection = {
  readonly messages: readonly RoomMessageSignal[];
  readonly observedAtUnixMs: number;
  readonly readOnlyFederatedEvents: readonly ReadOnlyFederatedEvent[];
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
