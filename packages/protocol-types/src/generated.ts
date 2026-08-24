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
  readonly downloadUrl?: string;
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

export type HandoffRequestEvent = {
  readonly actor: ActorRef;
  readonly content: ContentRef;
  readonly correlationId: string;
  readonly createdAt: string;
  readonly eventType: "org.agentroom.handoff.request.v1";
  readonly expiresAt: string;
  readonly id: string;
  readonly permissions: ReadonlyArray<HandoffPermission>;
  readonly schemaVersion: "1.0";
  readonly signature: string;
  readonly targetInstanceId: string;
} & Readonly<Record<string, unknown>>;

export type MessagePreviewEvent = {
  readonly actor: ActorRef;
  readonly content: ContentRef;
  readonly correlationId: string;
  readonly createdAt: string;
  readonly eventType: "org.agentroom.message.preview.v1";
  readonly id: string;
  readonly preview: string;
  readonly roomId: string;
  readonly schemaVersion: "1.0";
  readonly signature: string;
} & Readonly<Record<string, unknown>>;

export type Provenance = "human" | "human_confirmed_agent" | "autonomous_agent";
