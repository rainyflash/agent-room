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
    const firstAgent = agents.value[0];
    const selectedAgent =
      firstAgent === undefined ? await this.gateway.ensureDefaultAgent() : ok(firstAgent);
    if (!selectedAgent.ok) return selectedAgent;
    const lobby = selectPublicLobby(lobbies.value, preferredLocale);
    if (lobby === null) {
      return err({ code: 'onboarding.public_lobby_unavailable', retryable: true });
    }
    return ok({ agent: selectedAgent.value, lobby, reusedExistingAgent });
  }
}
