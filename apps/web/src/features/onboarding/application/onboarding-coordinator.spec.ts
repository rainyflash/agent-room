import { describe, expect, it, vi } from 'vitest';

import { OnboardingCoordinator } from '@/features/onboarding/application/onboarding-coordinator';
import type {
  OnboardingAgent,
  OnboardingGateway,
  PublicLobby,
} from '@/features/onboarding/domain/onboarding';
import { err, ok } from '@/shared/result';

const agent: OnboardingAgent = {
  agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
  avatarContentId: null,
  description: '',
  displayName: 'First Agent',
  matrixUserId: '@agent:matrix.test',
  registeredAtUnixMs: 1,
  slug: 'first-agent',
  visibility: 'private',
};
const lobby: PublicLobby = {
  activeInstanceCount: 1,
  catalogId: '0198b601-77a2-7f41-b4f4-940f291951b8',
  description: '',
  language: 'en',
  name: 'Public lobby',
  onlineAgentCount: 2,
  slug: 'public',
};

describe('首次引导协调器', () => {
  it('已有 Agent 时直接复用，绝不调用创建端点', async () => {
    const gateway = fixtureGateway([agent]);
    const result = await new OnboardingCoordinator(gateway).bootstrap('en-US');

    expect(result).toEqual(ok({ agent, lobby, reusedExistingAgent: true }));
    expect(gateway.ensureDefaultAgent.mock.calls).toHaveLength(0);
  });

  it('空账户只调用服务端幂等默认 Agent 端点一次', async () => {
    const gateway = fixtureGateway([]);
    const result = await new OnboardingCoordinator(gateway).bootstrap('en');

    expect(result).toEqual(ok({ agent, lobby, reusedExistingAgent: false }));
    expect(gateway.ensureDefaultAgent.mock.calls).toHaveLength(1);
  });

  it('公共大厅不存在时显式失败而不发明目录项', async () => {
    const gateway = fixtureGateway([agent], []);
    const result = await new OnboardingCoordinator(gateway).bootstrap('en');

    expect(result).toEqual(err({ code: 'onboarding.public_lobby_unavailable', retryable: true }));
  });
});

function fixtureGateway(
  agents: readonly OnboardingAgent[],
  lobbies: readonly PublicLobby[] = [lobby],
): OnboardingGateway & { ensureDefaultAgent: ReturnType<typeof vi.fn> } {
  return {
    ensureDefaultAgent: vi.fn(() => Promise.resolve(ok(agent))),
    listAgents: vi.fn(() => Promise.resolve(ok(agents))),
    listPublicLobbies: vi.fn(() => Promise.resolve(ok(lobbies))),
  };
}
