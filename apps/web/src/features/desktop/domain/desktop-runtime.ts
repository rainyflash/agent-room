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

export const bridgeRuntimeSchema = z
  .object({
    lifecycle: bridgeLifecycleSchema,
    authorization: authorizationPromptSchema.nullable(),
  })
  .strict();

export const desktopAgentTargetSchema = z
  .object({
    agentId: z.string().uuid(),
    publicLobbyCatalogId: z.string().uuid(),
    lobbyLanguage: z.string().trim().min(1).max(35),
  })
  .strict();
export type DesktopAgentTarget = z.infer<typeof desktopAgentTargetSchema>;

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
  })
  .strict();

export type BridgeRuntime = z.infer<typeof bridgeRuntimeSchema>;
export type BridgePhase = z.infer<typeof bridgePhaseSchema>;
export type DesktopDeepLink = z.infer<typeof desktopDeepLinkSchema>;
export type DesktopRuntimeSnapshot = z.infer<typeof desktopRuntimeSnapshotSchema>;

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

export type DesktopRuntimeEventHandlers = {
  readonly onDeepLink: (target: DesktopDeepLink) => void;
  readonly onFailure: (failure: DesktopRuntimeFailure) => void;
  readonly onRuntimeChanged: (runtime: BridgeRuntime) => void;
};

export type DesktopRuntimeGateway = {
  isAvailable(): boolean;
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
  configureAgentRuntime(
    target: DesktopAgentTarget,
  ): Promise<Result<DesktopAgentTarget, DesktopRuntimeFailure>>;
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
