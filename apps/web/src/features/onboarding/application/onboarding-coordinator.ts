import type {
  OnboardingBootstrap,
  OnboardingFailure,
  OnboardingGateway,
} from '@/features/onboarding/domain/onboarding';
import { selectPublicLobby } from '@/features/onboarding/domain/onboarding';
import { err, ok, type Result } from '@/shared/result';

export class OnboardingCoordinator {
  constructor(private readonly gateway: OnboardingGateway) {}

  async bootstrap(
    preferredLocale: string,
  ): Promise<Result<OnboardingBootstrap, OnboardingFailure>> {
    const [agents, lobbies] = await Promise.all([
      this.gateway.listAgents(),
      this.gateway.listPublicLobbies(),
    ]);
    if (!agents.ok) return agents;
    if (!lobbies.ok) return lobbies;

    const reusedExistingAgent = agents.value.length > 0;
    let selectedAgent = agents.value[0];
    if (selectedAgent === undefined) {
      const ensured = await this.gateway.ensureDefaultAgent();
      if (!ensured.ok) return ensured;
      selectedAgent = ensured.value;
    }
    if (selectedAgent === undefined) {
      return err({ code: 'onboarding.agent_resolution_failed', retryable: true });
    }
    const lobby = selectPublicLobby(lobbies.value, preferredLocale);
    if (lobby === null) {
      return err({ code: 'onboarding.public_lobby_unavailable', retryable: true });
    }
    return ok({ agent: selectedAgent, lobby, reusedExistingAgent });
  }
}
