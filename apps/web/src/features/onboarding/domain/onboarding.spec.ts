import { describe, expect, it } from 'vitest';

import {
  projectOnboardingPhase,
  selectPublicLobby,
  targetFor,
  targetMatches,
  type OnboardingAgent,
  type PublicLobby,
} from '@/features/onboarding/domain/onboarding';

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

const lobbies: readonly PublicLobby[] = [
  lobby('0198b601-77a2-7f41-b4f4-940f291951b8', 'English', 'en', 30),
  lobby('0198b601-77a3-74f1-b4f4-940f291951b9', '中文', 'zh-CN', 12),
];

describe('首次引导领域投影', () => {
  it('优先选择精确语言，再选择同语系，最后保留服务端排序', () => {
    expect(selectPublicLobby(lobbies, 'zh-CN')?.name).toBe('中文');
    expect(selectPublicLobby(lobbies, 'zh-TW')?.name).toBe('中文');
    expect(selectPublicLobby(lobbies, 'fr-FR')?.name).toBe('English');
  });

  it('只在账户、Agent 和本机目标事实一致后报告就绪', () => {
    expect(
      projectOnboardingPhase({
        accountReady: true,
        bootstrapFailed: false,
        bootstrapReady: true,
        bridgePhase: 'ready',
        desktopAvailable: true,
        targetMatches: false,
      }),
    ).toBe('configuring-runtime');
    expect(
      projectOnboardingPhase({
        accountReady: true,
        bootstrapFailed: false,
        bootstrapReady: true,
        bridgePhase: 'ready',
        desktopAvailable: true,
        targetMatches: true,
      }),
    ).toBe('ready');
  });

  it('桌面目标由权威 Agent 与真实大厅组合且可精确恢复', () => {
    const target = targetFor(agent, lobbies[1]!, 'en');
    expect(target).toEqual({
      agentId: agent.agentId,
      lobbyLanguage: 'zh-CN',
      publicLobbyCatalogId: lobbies[1]?.catalogId,
    });
    expect(targetMatches(target, target)).toBe(true);
    expect(targetMatches(null, target)).toBe(false);
  });
});

function lobby(catalogId: string, name: string, language: string, online: number): PublicLobby {
  return {
    activeInstanceCount: 1,
    catalogId,
    description: '',
    language,
    name,
    onlineAgentCount: online,
    slug: name.toLowerCase(),
  };
}
