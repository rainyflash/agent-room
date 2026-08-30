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

export type OnboardingPhase = 'checking-account' | 'checking-agents' | 'failed' | 'ready';

export type OnboardingFacts = {
  readonly accountReady: boolean;
  readonly bootstrapFailed: boolean;
  readonly bootstrapReady: boolean;
};

export type OnboardingRuntimePhase =
  | 'authorization-required'
  | 'configuration-required'
  | 'connecting'
  | 'failed'
  | 'optional'
  | 'ready';

export type OnboardingRuntimeFacts = {
  readonly bridgePhase: BridgePhase | null;
  readonly desktopAvailable: boolean;
  readonly runtimeSessionReady: boolean;
  readonly targetMatches: boolean;
};

export function projectOnboardingPhase(facts: OnboardingFacts): OnboardingPhase {
  return onboardingPhaseRules.find((rule) => rule.matches(facts))?.phase ?? 'ready';
}

export function projectOnboardingRuntimePhase(
  facts: OnboardingRuntimeFacts,
): OnboardingRuntimePhase {
  return runtimePhaseRules.find((rule) => rule.matches(facts))?.phase ?? 'connecting';
}

type PhaseRule<TFacts, TPhase extends string> = Readonly<{
  matches: (facts: TFacts) => boolean;
  phase: TPhase;
}>;

const onboardingPhaseRules: readonly PhaseRule<OnboardingFacts, OnboardingPhase>[] = [
  { matches: (facts) => !facts.accountReady, phase: 'checking-account' },
  { matches: (facts) => facts.bootstrapFailed, phase: 'failed' },
  { matches: (facts) => !facts.bootstrapReady, phase: 'checking-agents' },
];

const runtimePhaseRules: readonly PhaseRule<OnboardingRuntimeFacts, OnboardingRuntimePhase>[] = [
  { matches: (facts) => !facts.desktopAvailable, phase: 'optional' },
  { matches: (facts) => !facts.targetMatches, phase: 'configuration-required' },
  { matches: (facts) => facts.bridgePhase === 'halted', phase: 'failed' },
  {
    matches: (facts) => facts.bridgePhase === 'authorization_required',
    phase: 'authorization-required',
  },
  {
    matches: (facts) => facts.bridgePhase === 'ready' && facts.runtimeSessionReady,
    phase: 'ready',
  },
  { matches: (facts) => facts.bridgePhase === 'ready', phase: 'failed' },
];

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
