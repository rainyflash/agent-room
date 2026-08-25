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
  })
  .strict();

export type BridgeRuntime = z.infer<typeof bridgeRuntimeSchema>;
export type BridgePhase = z.infer<typeof bridgePhaseSchema>;
export type DesktopDeepLink = z.infer<typeof desktopDeepLinkSchema>;
export type DesktopRuntimeSnapshot = z.infer<typeof desktopRuntimeSnapshotSchema>;

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
