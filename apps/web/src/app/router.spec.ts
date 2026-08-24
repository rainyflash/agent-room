import { describe, expect, it } from 'vitest';

import {
  lobbySearchWithAgent,
  normalizeConnectSearch,
  normalizeLobbySearch,
} from '@/shared/routing/route-state';

describe('路由状态规范化', () => {
  it('丢弃开放跳转和无效可选情境', () => {
    expect(normalizeConnectSearch({ returnTo: '//evil.example' })).toEqual({});
    expect(
      normalizeLobbySearch({
        agent: 'valid-agent',
        directory: 'closed',
        message: 'bad message',
      }),
    ).toEqual({ agent: 'valid-agent' });
  });

  it('保留可恢复的大厅情境', () => {
    expect(
      normalizeLobbySearch({
        agent: 'agent-01',
        directory: 'open',
        message: '$event:matrix.example',
      }),
    ).toEqual({
      agent: 'agent-01',
      directory: 'open',
      message: '$event:matrix.example',
    });
  });

  it('更新选中 Agent 时物理删除空查询字段', () => {
    expect(
      lobbySearchWithAgent(
        { agent: 'agent-01', directory: 'open', message: '$event:matrix.example' },
        null,
      ),
    ).toEqual({ directory: 'open', message: '$event:matrix.example' });
    expect(lobbySearchWithAgent({ directory: 'open' }, 'agent-02')).toEqual({
      agent: 'agent-02',
      directory: 'open',
    });
  });
});
