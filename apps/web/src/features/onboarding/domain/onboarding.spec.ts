import { describe, expect, it } from 'vitest';

import {
  projectOnboardingPhase,
  projectOnboardingRuntimePhase,
  targetFor,
  targetMatches,
  type OnboardingAgent,
} from '@/features/onboarding/domain/onboarding';
import type { PublicRoomSummary } from '@/features/room-directory/domain/public-room-directory';

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

const chineseLobby = lobby('0198b601-77a3-74f1-b4f4-940f291951b9', '中文', 'zh-CN', 12);

describe('首次引导领域投影', () => {
  it('云端引导只依赖账户与服务端事实，不被可选 Bridge 阻塞', () => {
    expect(
      projectOnboardingPhase({
        accountReady: true,
        bootstrapFailed: false,
        bootstrapReady: true,
      }),
    ).toBe('ready');
    expect(
      projectOnboardingPhase({
        accountReady: true,
        bootstrapFailed: true,
        bootstrapReady: false,
      }),
    ).toBe('failed');
  });

  it('本机 Runtime 独立投影，Bridge 停机不会污染云端引导状态', () => {
    expect(
      projectOnboardingRuntimePhase({
        bridgePhase: 'halted',
        desktopAvailable: true,
        runtimeSessionReady: false,
        targetMatches: true,
      }),
    ).toBe('failed');
    expect(
      projectOnboardingRuntimePhase({
        bridgePhase: 'ready',
        desktopAvailable: true,
        runtimeSessionReady: true,
        targetMatches: true,
      }),
    ).toBe('ready');
    expect(
      projectOnboardingRuntimePhase({
        bridgePhase: null,
        desktopAvailable: false,
        runtimeSessionReady: false,
        targetMatches: false,
      }),
    ).toBe('optional');
  });

  it('桌面目标由权威 Agent 与真实大厅组合且可精确恢复', () => {
    const target = targetFor(agent, chineseLobby, 'en');
    expect(target).toEqual({
      agentId: agent.agentId,
      lobbyLanguage: 'zh-CN',
      publicLobbyCatalogId: chineseLobby.catalogId,
    });
    expect(targetMatches(target, target)).toBe(true);
    expect(targetMatches(null, target)).toBe(false);
  });
});

function lobby(
  catalogId: string,
  name: string,
  language: string,
  online: number,
): PublicRoomSummary {
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
