import { z } from 'zod';

import type { BridgePhase, DesktopAgentTarget } from '@/features/desktop/domain/desktop-runtime';
import type { Result } from '@/shared/result';
import { uuidV7Schema } from '@/shared/validation/identifiers';

export const onboardingAgentSchema = z
  .object({
    agentId: uuidV7Schema,
    matrixUserId: z.string().regex(/^@[^:]+:.+$/u),
    slug: z.string().trim().min(1).max(96),
    displayName: z.string().trim().min(1).max(160),
    description: z.string().max(4_000),
    avatarContentId: uuidV7Schema.nullable(),
    visibility: z.enum(['private', 'public', 'unlisted']),
    registeredAtUnixMs: z.number().int().nonnegative(),
  })
  .strict();

export const onboardingAgentListSchema = z
  .object({ agents: z.array(onboardingAgentSchema) })
  .strict();

export const publicLobbySchema = z
  .object({
    catalogId: uuidV7Schema,
    slug: z.string().trim().min(1).max(96).nullable(),
    name: z.string().trim().min(1).max(160),
    description: z.string().max(4_000),
    language: z.string().trim().min(1).max(35).nullable(),
    activeInstanceCount: z.number().int().min(0).max(65_535),
    onlineAgentCount: z.number().int().nonnegative(),
  })
  .strict();

export const publicLobbyDirectorySchema = z
  .object({ lobbies: z.array(publicLobbySchema) })
  .strict();

export type OnboardingAgent = z.infer<typeof onboardingAgentSchema>;
export type PublicLobby = z.infer<typeof publicLobbySchema>;

export type OnboardingFailure = {
  readonly code: string;
  readonly retryable: boolean;
};

export type OnboardingGateway = {
  ensureDefaultAgent(): Promise<Result<OnboardingAgent, OnboardingFailure>>;
  listAgents(): Promise<Result<readonly OnboardingAgent[], OnboardingFailure>>;
  listPublicLobbies(): Promise<Result<readonly PublicLobby[], OnboardingFailure>>;
};

export type OnboardingBootstrap = {
  readonly agent: OnboardingAgent;
  readonly lobby: PublicLobby;
  readonly reusedExistingAgent: boolean;
};

export type OnboardingPhase =
  | 'authorizing-runtime'
  | 'checking-account'
  | 'checking-agents'
  | 'configuring-runtime'
  | 'failed'
  | 'ready'
  | 'runtime-required';

export type OnboardingFacts = {
  readonly accountReady: boolean;
  readonly bootstrapFailed: boolean;
  readonly bootstrapReady: boolean;
  readonly bridgePhase: BridgePhase | null;
  readonly desktopAvailable: boolean;
  readonly runtimeSessionReady: boolean;
  readonly targetMatches: boolean;
};

export function projectOnboardingPhase(facts: OnboardingFacts): OnboardingPhase {
  if (!facts.accountReady) return 'checking-account';
  if (facts.bootstrapFailed || facts.bridgePhase === 'halted') return 'failed';
  if (!facts.bootstrapReady) return 'checking-agents';
  if (!facts.desktopAvailable) return 'runtime-required';
  if (!facts.targetMatches) return 'configuring-runtime';
  if (facts.bridgePhase === 'authorization_required') return 'authorizing-runtime';
  if (facts.bridgePhase !== 'ready') return 'configuring-runtime';
  return facts.runtimeSessionReady ? 'ready' : 'failed';
}

export function selectPublicLobby(
  lobbies: readonly PublicLobby[],
  preferredLocale: string,
): PublicLobby | null {
  if (lobbies.length === 0) return null;
  const normalizedLocale = preferredLocale.trim().toLowerCase();
  const baseLanguage = normalizedLocale.split('-')[0];
  return (
    lobbies.find((lobby) => lobby.language?.toLowerCase() === normalizedLocale) ??
    lobbies.find((lobby) => lobby.language?.toLowerCase().split('-')[0] === baseLanguage) ??
    lobbies[0] ??
    null
  );
}

export function targetFor(
  agent: OnboardingAgent,
  lobby: PublicLobby,
  locale: string,
): DesktopAgentTarget {
  return {
    agentId: agent.agentId,
    lobbyLanguage: lobby.language ?? locale,
    publicLobbyCatalogId: lobby.catalogId,
  };
}

export function targetMatches(
  current: DesktopAgentTarget | null,
  expected: DesktopAgentTarget,
): boolean {
  return (
    current?.agentId === expected.agentId &&
    current.publicLobbyCatalogId === expected.publicLobbyCatalogId &&
    current.lobbyLanguage.toLowerCase() === expected.lobbyLanguage.toLowerCase()
  );
}
