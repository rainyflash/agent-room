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

/**
 * 解不开的原因，按用户能做什么来分：
 * 没收到密钥、发送方不肯给（这台设备还没验证）、发送方设备不可信、消息早于加入、其他。
 */
export type UndecryptableReason =
  'missing_key' | 'withheld' | 'untrusted_sender' | 'before_join' | 'other';

/** 这台设备解不开的加密事件。解开以后就从这里消失。 */
export type UndecryptableSummary = {
  readonly count: number;
  /** 按上面类型的先后排好、不重复。 */
  readonly reasons: readonly UndecryptableReason[];
  /** 发这些事件的 Matrix 用户，不重复。 */
  readonly senders: readonly string[];
};

/**
 * 找回解不开的消息走到了哪一步：已经请 Agent 重发、还在等；
 * 或者这台设备还没由主人签名，Agent 不会回答，要先验证这台设备。
 */
export type UndecryptableRecovery = 'idle' | 'requested' | 'needs_verification';

export type MessageRoomProjection = {
  readonly history?: { readonly canLoadMore: boolean; readonly limited: boolean };
  readonly messages: readonly RoomMessageSignal[];
  /**
   * 这份投影算出来的时间。房间没变时网关交回同一份投影，这个时间也不变，
   * 所以它不是“现在”；需要当前时间的地方自己取时钟。
   */
  readonly observedAtUnixMs: number;
  readonly readOnlyFederatedEvents: readonly ReadOnlyFederatedEvent[];
  readonly roomId: string;
  /** 没有解不开的事件时不给。 */
  readonly undecryptable?: UndecryptableSummary;
};

export type MessageFailureCode =
  'messages.matrix_unavailable' | 'messages.room_not_joined' | 'messages.projection_invalid';

export type MessageFailure = {
  readonly code: MessageFailureCode;
  readonly retryable: boolean;
};

export type MessageReadResult = Result<MessageRoomProjection, MessageFailure>;

export type MessageGateway = {
  loadOlder?(
    roomId: string,
  ): Promise<Result<void, { readonly code: string; readonly retryable: boolean }>>;
  read(roomId: string): MessageReadResult;
  subscribe(roomId: string, listener: () => void): () => void;
};
