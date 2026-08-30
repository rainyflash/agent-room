import { z } from 'zod';

import type { Result } from '@/shared/result';

const diagnosticCodeSchema = z
  .string()
  .min(1)
  .max(128)
  .regex(/^[a-z0-9._]+$/u);

export const bridgePhaseSchema = z.enum([
  'discovering',
  'starting',
  'authorization_required',
  'authorized',
  'ready',
  'retry_scheduled',
  'halted',
  'stopped',
]);

const bridgeLifecycleSchema = z
  .object({
    phase: bridgePhaseSchema,
    ownership: z.enum(['external', 'managed']).nullable(),
    diagnosticCode: diagnosticCodeSchema.nullable(),
    lastFailureCode: diagnosticCodeSchema.nullable(),
    automaticRestartCount: z.number().int().nonnegative(),
    nextRetryAtUnixMs: z.number().int().nonnegative().nullable(),
    lastExitCode: z.number().int().nullable(),
    changedAtUnixMs: z.number().int().nonnegative(),
  })
  .strict();

const authorizationPromptSchema = z
  .object({
    promptId: z.string().min(1).max(96),
    verificationHost: z.string().min(1).max(253),
    userCode: z.string().min(1).max(64),
    expiresAtUnixMs: z.number().int().positive(),
  })
  .strict();

export const bridgeAgentSessionSchema = z
  .object({
    agentId: z.uuid(),
    instanceId: z.uuid(),
    matrixRoomId: z
      .string()
      .min(4)
      .max(512)
      .regex(/^![^\s]+:[^\s]+$/u),
  })
  .strict();

export const bridgeRuntimeSchema = z
  .object({
    lifecycle: bridgeLifecycleSchema,
    authorization: authorizationPromptSchema.nullable(),
    session: bridgeAgentSessionSchema.nullable(),
  })
  .strict();

export const desktopAgentTargetSchema = z
  .object({
    agentId: z.uuid(),
    publicLobbyCatalogId: z.uuid(),
    lobbyLanguage: z.string().trim().min(1).max(35),
  })
  .strict();
export type DesktopAgentTarget = z.infer<typeof desktopAgentTargetSchema>;

const desktopLobbyAgentIdentitySchema = z
  .object({
    agentId: z.uuid(),
    displayName: z.string().trim().min(1).max(160),
    matrixUserId: z.string().min(4).max(512),
    avatarUrl: z.string().min(1).max(2_048).nullable(),
  })
  .strict();

const desktopLobbyActorSchema = z
  .object({
    agent: desktopLobbyAgentIdentitySchema,
    instanceId: z.uuid(),
    provenance: z.enum(['human', 'human_confirmed_agent', 'autonomous_agent']),
  })
  .strict();

const desktopLobbyContentSchema = z
  .object({
    contentId: z.uuid(),
    digestSha256: z.string().length(64),
    mediaType: z.string().min(1).max(255),
    sizeBytes: z.number().int().nonnegative(),
  })
  .strict();

const desktopLobbyIdentitySchema = z
  .object({
    agent: desktopLobbyAgentIdentitySchema,
    instanceId: z.uuid(),
    matrixDeviceId: z.string().min(1).max(255),
    roomId: z.string().min(4).max(512),
    connectionState: z.enum(['starting', 'ready', 'reconnecting', 'offline', 'shutting_down']),
    grantedCapabilities: z.array(z.string().min(1).max(128)).max(128),
  })
  .strict();

const desktopLobbyPresenceSchema = z
  .object({
    roomId: z.string().min(4).max(512),
    agent: desktopLobbyAgentIdentitySchema,
    instanceId: z.uuid(),
    status: z.enum(['offline', 'idle', 'working', 'waiting_input', 'blocked', 'completed']),
    observedAtUnixMs: z.number().int().nonnegative(),
    leaseExpiresAtUnixMs: z.number().int().nonnegative(),
  })
  .strict();

const desktopLobbyMessageSchema = z
  .object({
    messageId: z.uuid(),
    eventId: z.string().min(1).max(512),
    roomId: z.string().min(4).max(512),
    actor: desktopLobbyActorSchema,
    createdAtUnixMs: z.number().int().nonnegative(),
    title: z.string().trim().min(1).max(240),
    summary: z.string().trim().max(2_000),
    content: desktopLobbyContentSchema,
    language: z.string().trim().min(1).max(35).nullable(),
    sensitivity: z.enum(['normal', 'sensitive', 'restricted']),
    riskFlags: z.array(z.string().min(1).max(128)).max(64),
  })
  .strict();

export const desktopLobbySnapshotSchema = z
  .object({
    identity: desktopLobbyIdentitySchema,
    agents: z.array(desktopLobbyPresenceSchema).max(250),
    messages: z.array(desktopLobbyMessageSchema).max(100),
    nextCursor: z.string().min(1).max(512).nullable(),
    observedAtUnixMs: z.number().int().nonnegative(),
  })
  .strict();
export type DesktopLobbySnapshot = z.infer<typeof desktopLobbySnapshotSchema>;

const routeSegmentSchema = z
  .string()
  .min(1)
  .max(255)
  .regex(/^[A-Za-z0-9_.:!-]+$/u);

export const desktopDeepLinkSchema = z
  .object({
    kind: z.literal('lobby'),
    route: z.string().min(1).max(800),
  })
  .strict()
  .superRefine((target, context) => {
    if (parseLobbyDeepLinkRoute(target.route) === null) {
      context.addIssue({ code: 'custom', message: 'desktop.deep_link.invalid' });
    }
  });

export const desktopRuntimeSnapshotSchema = z
  .object({
    bridge: bridgeRuntimeSchema,
    autostartEnabled: z.boolean(),
    platform: z.enum(['windows', 'macos', 'linux', 'unknown']),
    deepLink: desktopDeepLinkSchema.nullable(),
    updatesConfigured: z.boolean(),
    agentTarget: desktopAgentTargetSchema.nullable(),
    manualHostConfiguration: z
      .object({
        args: z.array(z.string().max(2_048)).max(32),
        command: z.string().min(1).max(2_048),
        serverName: z.literal('agent_room'),
        transport: z.literal('stdio'),
      })
      .strict(),
  })
  .strict();

export type BridgeRuntime = z.infer<typeof bridgeRuntimeSchema>;
export type BridgeAgentSession = z.infer<typeof bridgeAgentSessionSchema>;
export type BridgePhase = z.infer<typeof bridgePhaseSchema>;
export type DesktopDeepLink = z.infer<typeof desktopDeepLinkSchema>;
export type DesktopRuntimeSnapshot = z.infer<typeof desktopRuntimeSnapshotSchema>;
export type ManualHostConfiguration = DesktopRuntimeSnapshot['manualHostConfiguration'];

export const releaseUpdateChannelSchema = z.enum(['stable', 'testing']);
export type ReleaseUpdateChannel = z.infer<typeof releaseUpdateChannelSchema>;

export const releaseUpdateCheckSchema = z
  .object({
    available: z.boolean(),
    channel: releaseUpdateChannelSchema,
    currentVersion: z.string().min(1).max(64),
    targetVersion: z.string().min(1).max(64),
    sequence: z.number().int().nonnegative(),
    rollback: z.boolean(),
  })
  .strict();
export type ReleaseUpdateCheck = z.infer<typeof releaseUpdateCheckSchema>;

export const agentHostKindSchema = z.enum(['codex', 'claude-code', 'cursor']);
export type AgentHostKind = z.infer<typeof agentHostKindSchema>;

export const agentHostDetectionSchema = z
  .object({
    host: agentHostKindSchema,
    installed: z.boolean(),
    configurable: z.boolean(),
    mechanism: z.string().min(1).max(64),
    diagnosticCode: diagnosticCodeSchema,
  })
  .strict();
export type AgentHostDetection = z.infer<typeof agentHostDetectionSchema>;

export const agentHostPlanSchema = z
  .object({
    host: agentHostKindSchema,
    action: z.enum(['create', 'replace', 'unchanged', 'unavailable']),
    target: z.string().min(1).max(256),
    originalDigest: z.string().length(64),
    desiredDigest: z.string().length(64),
    summaryCode: diagnosticCodeSchema,
  })
  .strict();
export type AgentHostPlan = z.infer<typeof agentHostPlanSchema>;

export const agentHostApplyReceiptSchema = z
  .object({
    host: agentHostKindSchema,
    changed: z.boolean(),
    resultingDigest: z.string().length(64),
  })
  .strict();

export type DesktopRuntimeFailure = {
  readonly code: string;
  readonly retryable: boolean;
};

export const desktopHumanSessionChangedSchema = z
  .object({
    returnPath: z.string().min(1).max(2_048),
    session: z
      .object({
        authenticatedAtUnixMs: z.number().int().nonnegative(),
        displayName: z.string().trim().min(1).max(160),
        expiresAtUnixMs: z.number().int().positive(),
        locale: z.string().trim().min(2).max(35),
        matrixUserId: z.string().regex(/^@[^:]+:.+$/u),
        principalId: z.uuid(),
        recentlyAuthenticated: z.boolean(),
      })
      .strict(),
  })
  .strict()
  .superRefine((value, context) => {
    if (!value.returnPath.startsWith('/') || value.returnPath.startsWith('//')) {
      context.addIssue({ code: 'custom', message: 'desktop.human_session.return_path_invalid' });
    }
  });
export type DesktopHumanSessionChanged = z.infer<typeof desktopHumanSessionChangedSchema>;
export type DesktopAuthenticationIntent = 'register' | 'sign-in';

export type DesktopRuntimeEventHandlers = {
  readonly onDeepLink: (target: DesktopDeepLink) => void;
  readonly onFailure: (failure: DesktopRuntimeFailure) => void;
  readonly onRuntimeChanged: (runtime: BridgeRuntime) => void;
};

export type DesktopRuntimeGateway = {
  isAvailable(): boolean;
  beginHumanAuthentication(
    returnPath: string,
    intent: DesktopAuthenticationIntent,
  ): Promise<Result<DesktopHumanSessionChanged, DesktopRuntimeFailure>>;
  clearHumanSession(): Promise<Result<void, DesktopRuntimeFailure>>;
  snapshot(): Promise<Result<DesktopRuntimeSnapshot, DesktopRuntimeFailure>>;
  retryBridge(): Promise<Result<BridgeRuntime, DesktopRuntimeFailure>>;
  setAutostart(enabled: boolean): Promise<Result<boolean, DesktopRuntimeFailure>>;
  openAuthorization(promptId: string): Promise<Result<void, DesktopRuntimeFailure>>;
  checkUpdate(
    channel: ReleaseUpdateChannel,
  ): Promise<Result<ReleaseUpdateCheck, DesktopRuntimeFailure>>;
  installUpdate(
    channel: ReleaseUpdateChannel,
    expectedSequence: number,
  ): Promise<Result<void, DesktopRuntimeFailure>>;
  bootstrapDefaultAgent(
    preferredLanguage: string | null,
  ): Promise<Result<DesktopAgentTarget, DesktopRuntimeFailure>>;
  configureAgentRuntime(
    target: DesktopAgentTarget,
  ): Promise<Result<DesktopAgentTarget, DesktopRuntimeFailure>>;
  readLobby(): Promise<Result<DesktopLobbySnapshot, DesktopRuntimeFailure>>;
  detectHosts?(): Promise<Result<readonly AgentHostDetection[], DesktopRuntimeFailure>>;
  planHost?(host: AgentHostKind): Promise<Result<AgentHostPlan, DesktopRuntimeFailure>>;
  applyHost?(
    host: AgentHostKind,
    expectedOriginalDigest: string,
  ): Promise<Result<void, DesktopRuntimeFailure>>;
  subscribe(
    handlers: DesktopRuntimeEventHandlers,
  ): Promise<Result<() => void, DesktopRuntimeFailure>>;
};

export type LobbyDeepLinkNavigation =
  | { readonly kind: 'catalog'; readonly catalogId: string }
  | { readonly kind: 'instance'; readonly catalogId: string; readonly roomId: string };

export function parseLobbyDeepLinkRoute(route: string): LobbyDeepLinkNavigation | null {
  const segments = route.split('/').filter(Boolean);
  if (segments[0] !== 'lobby') {
    return null;
  }
  if (segments.length === 2) {
    const catalog = routeSegmentSchema.safeParse(segments[1]);
    return catalog.success ? { kind: 'catalog', catalogId: catalog.data } : null;
  }
  if (segments.length === 4 && segments[2] === 'instance') {
    const catalog = routeSegmentSchema.safeParse(segments[1]);
    const room = routeSegmentSchema.safeParse(segments[3]);
    if (catalog.success && room.success) {
      return { kind: 'instance', catalogId: catalog.data, roomId: room.data };
    }
  }
  return null;
}
