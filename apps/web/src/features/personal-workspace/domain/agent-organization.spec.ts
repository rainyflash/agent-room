import { describe, expect, it } from 'vitest';
import type { LobbyAgent } from '@/features/lobby/domain/lobby';
import { emptyWorkspace, workspaceIndex } from './workspace-document';
import { organizeAgents, parseProjectTags, projectTags } from './agent-organization';

function agent(id: string, displayName = 'Same'): LobbyAgent {
  return {
    agentId: id,
    displayName,
    instanceIds: [],
    matrixUserId: `@${id}:example.org`,
    status: 'idle',
    statusExpiresAtUnixMs: 0,
    trust: 'unknown',
    visibility: 'coarse',
  };
}
describe('personal agent organization', () => {
  it('keeps ties stable, honors explicit tags and orders recent contacts by real activity', () => {
    const agents = [agent('c'), agent('b'), agent('a')];
    const prefs = workspaceIndex(emptyWorkspace);
    prefs.favorites.add('c');
    prefs.tags.set('b', ['Release']);
    prefs.tags.set('c', ['Design']);
    prefs.recent.set('b', 3);
    prefs.recent.set('c', 1);
    expect(organizeAgents(agents, prefs, 'all', '').map((item) => item.agentId)).toEqual([
      'c',
      'a',
      'b',
    ]);
    expect(
      organizeAgents(agents.toReversed(), prefs, 'all', '').map((item) => item.agentId),
    ).toEqual(['c', 'a', 'b']);
    expect(organizeAgents(agents, prefs, 'recent', '').map((item) => item.agentId)).toEqual([
      'b',
      'c',
    ]);
    expect(organizeAgents(agents, prefs, 'favorites', 'Release')).toEqual([]);
    expect(projectTags(agents, prefs)).toEqual(['Design', 'Release']);
  });
  it('normalizes user labels and rejects oversized metadata', () => {
    expect(parseProjectTags(' 项目 A，Release, 项目 A, ')).toEqual(['项目 A', 'Release']);
    expect(parseProjectTags('')).toEqual([]);
    expect(parseProjectTags('x'.repeat(33))).toBeNull();
    expect(
      parseProjectTags(Array.from({ length: 13 }, (_, index) => String(index)).join(',')),
    ).toBeNull();
  });
});
