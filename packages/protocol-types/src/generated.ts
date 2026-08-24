// 本文件由 tools/protocol-codegen.ts 生成，禁止手工修改。

export type ActorRef = {
  readonly agent: AgentRef;
  readonly instanceId: string;
  readonly provenance: Provenance;
} & Readonly<Record<string, unknown>>;

export type AgentRef = {
  readonly agentId: string;
  readonly avatarUrl?: string;
  readonly displayName: string;
  readonly matrixUserId: string;
} & Readonly<Record<string, unknown>>;

export type AgentStatusEvent = {
  readonly actor: ActorRef;
  readonly correlationId: string;
  readonly createdAt: string;
  readonly eventType: "org.agentroom.agent.status.v1";
  readonly id: string;
  readonly leaseExpiresAt: string;
  readonly progress?: number;
  readonly schemaVersion: "1.0";
  readonly signature: string;
  readonly startedAt?: string;
  readonly status: AgentWorkStatus;
  readonly taskSummary?: string;
  readonly visibility: AgentStatusVisibility;
} & Readonly<Record<string, unknown>>;

export type AgentStatusVisibility = "coarse" | "detailed";

export type AgentWorkStatus = "offline" | "idle" | "working" | "waiting_input" | "blocked" | "completed";

export type CapabilityManifest = {
  readonly eventTypes: ReadonlyArray<string>;
  readonly features: ReadonlyArray<string>;
  readonly protocolVersions: ReadonlyArray<string>;
  readonly schemaVersion: "1.0";
} & Readonly<Record<string, unknown>>;

export type ContentRef = {
  readonly contentId: string;
  readonly digestSha256: string;
  readonly fetchMode: "on_demand";
  readonly mediaType: string;
  readonly sizeBytes: number;
} & Readonly<Record<string, unknown>>;

export type ErrorCategory = "validation" | "authentication" | "authorization" | "conflict" | "transient" | "unknown_commit" | "dependency_unavailable" | "incompatible_version";

export type ErrorEnvelope = {
  readonly category: ErrorCategory;
  readonly code: string;
  readonly correlationId: string;
  readonly details: Readonly<Record<string, unknown>>;
  readonly message: string;
  readonly retryable: boolean;
  readonly retryAfterSeconds?: number;
} & Readonly<Record<string, unknown>>;

export type HandoffPermission = "read_text" | "read_attachments" | "include_metadata";

export type HandoffPurpose = "inspect" | "summarize" | "reply_draft";

export type HandoffReceiptEvent = {
  readonly actor: ActorRef;
  readonly correlationId: string;
  readonly createdAt: string;
  readonly eventType: "org.agentroom.handoff.receipt.v1";
  readonly failureCode?: string;
  readonly id: string;
  readonly requesterInstanceId: string;
  readonly schemaVersion: "1.0";
  readonly signature: string;
  readonly status: HandoffReceiptStatus;
} & Readonly<Record<string, unknown>>;

export type HandoffReceiptStatus = "delivered" | "consumed" | "declined" | "revoked" | "expired" | "failed";

export type HandoffRequestEvent = {
  readonly actor: ActorRef;
  readonly approvedAt: string;
  readonly approvedByPrincipalId: string;
  readonly content: ContentRef;
  readonly correlationId: string;
  readonly createdAt: string;
  readonly eventType: "org.agentroom.handoff.request.v1";
  readonly expiresAt: string;
  readonly id: string;
  readonly permissions: ReadonlyArray<HandoffPermission>;
  readonly purpose: HandoffPurpose;
  readonly riskFlags: ReadonlyArray<string>;
  readonly schemaVersion: "1.0";
  readonly signature: string;
  readonly source: HandoffSource;
  readonly targetAgentId: string;
  readonly targetInstanceId: string;
} & Readonly<Record<string, unknown>>;

export type HandoffSource = {
  readonly actor: ActorRef;
  readonly eventId: string;
  readonly messageId: string;
  readonly roomId: string;
} & Readonly<Record<string, unknown>>;

export type MessagePreview = {
  readonly contentType: string;
  readonly language?: string;
  readonly riskFlags: ReadonlyArray<string>;
  readonly sensitivity: MessageSensitivity;
  readonly summary: string;
  readonly title: string;
} & Readonly<Record<string, unknown>>;

export type MessagePreviewEvent = {
  readonly actor: ActorRef;
  readonly content: ContentRef;
  readonly correlationId: string;
  readonly createdAt: string;
  readonly eventType: "org.agentroom.message.preview.v1";
  readonly id: string;
  readonly preview: MessagePreview;
  readonly relation?: MessageRelation;
  readonly roomId: string;
  readonly schemaVersion: "1.0";
  readonly signature: string;
} & Readonly<Record<string, unknown>>;

export type MessageRelation = {
  readonly kind: "reply";
  readonly targetMessageId: string;
} & Readonly<Record<string, unknown>>;

export type MessageRevisionEvent = {
  readonly actor: ActorRef;
  readonly content?: ContentRef;
  readonly correlationId: string;
  readonly createdAt: string;
  readonly eventType: "org.agentroom.message.revision.v1";
  readonly id: string;
  readonly kind: MessageRevisionKind;
  readonly preview?: MessagePreview;
  readonly roomId: string;
  readonly schemaVersion: "1.0";
  readonly signature: string;
  readonly targetMessageId: string;
} & Readonly<Record<string, unknown>>;

export type MessageRevisionKind = "replace" | "redact" | "moderate";

export type MessageSensitivity = "normal" | "sensitive" | "restricted";

export type Provenance = "human" | "human_confirmed_agent" | "autonomous_agent";
