import { describe, expect, it } from 'vitest';

import {
  lobbySearchWithAgent,
  lobbySearchWithDirectSession,
  lobbySearchWithLayout,
  lobbySearchWithMessage,
  lobbySearchWithView,
  lobbySearchWithRoomPanel,
  normalizeConnectSearch,
  normalizeLobbySearch,
  normalizeWorkspaceSearch,
} from '@/shared/routing/route-state';

describe('路由状态规范化', () => {
  it('丢弃开放跳转和无效可选情境', () => {
    expect(normalizeConnectSearch({ returnTo: '//evil.example' })).toEqual({});
    expect(
      normalizeLobbySearch({
        agent: 'valid-agent',
        direct: 'bad direct',
        directory: 'closed',
        message: 'bad message',
      }),
    ).toEqual({ agent: 'valid-agent' });
  });

  it('保留可恢复的大厅情境', () => {
    expect(
      normalizeLobbySearch({
        agent: 'agent-01',
        direct: '01990d9e-8400-7000-8000-000000000020',
        directory: 'open',
        message: '$event:matrix.example',
      }),
    ).toEqual({
      agent: 'agent-01',
      direct: '01990d9e-8400-7000-8000-000000000020',
      directory: 'open',
      message: '$event:matrix.example',
    });
  });

  it('只保留合法的工作区 Agent 选择', () => {
    expect(normalizeWorkspaceSearch({ agent: '0198b601-77a1-7bb8-83eb-a8fe68c97e44' })).toEqual({
      agent: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
    });
    expect(normalizeWorkspaceSearch({ agent: 'bad agent' })).toEqual({});
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

  it('用 URL 中的直接会话替换互斥的大厅选择', () => {
    expect(
      lobbySearchWithDirectSession(
        { agent: 'agent-01', directory: 'open', message: '$event:matrix.example' },
        '01990d9e-8400-7000-8000-000000000020',
      ),
    ).toEqual({
      direct: '01990d9e-8400-7000-8000-000000000020',
      directory: 'open',
    });
    expect(
      lobbySearchWithMessage(
        { direct: '01990d9e-8400-7000-8000-000000000020' },
        '$event:matrix.example',
      ),
    ).toEqual({
      direct: '01990d9e-8400-7000-8000-000000000020',
      message: '$event:matrix.example',
    });
    expect(
      lobbySearchWithDirectSession(
        { direct: '01990d9e-8400-7000-8000-000000000020', message: '$event:matrix.example' },
        null,
      ),
    ).toEqual({});
  });
});

describe('大厅视图路由', () => {
  it('恢复有效视图并忽略无效值和私聊空间视图', () => {
    expect(normalizeLobbySearch({ view: 'resources' })).toEqual({ view: 'resources' });
    expect(normalizeLobbySearch({ view: 'space' })).toEqual({});
    expect(normalizeLobbySearch({ view: 'unknown' })).toEqual({});
    expect(normalizeLobbySearch({ direct: 'private-room', view: 'space' })).toEqual({
      direct: 'private-room',
    });
  });
  it('关闭面板回到房间，打开大厅面板退出私聊', () => {
    expect(
      lobbySearchWithView(
        { direct: 'private-room', message: '$message', view: 'resources' },
        'space',
      ),
    ).toEqual({});
    expect(
      lobbySearchWithRoomPanel(
        { direct: 'private-room', agent: 'agent-01', message: '$message', directory: 'open' },
        'conversation',
      ),
    ).toEqual({ view: 'conversation', directory: 'open' });
    expect(normalizeLobbySearch({ view: 'conversation' })).toEqual({ view: 'conversation' });
  });
  it('检查资料时保留视图，进入私聊回到对话', () => {
    expect(lobbySearchWithMessage({ view: 'resources' }, '$message')).toEqual({
      view: 'resources',
      message: '$message',
    });
    expect(lobbySearchWithDirectSession({}, 'private-room')).toEqual({
      direct: 'private-room',
    });
  });
});

describe('Agent 表格展开成整页的路由', () => {
  it('地址里认得 view=agents；从右栏展开时别的选择都放开，版面留着', () => {
    expect(normalizeLobbySearch({ view: 'agents' })).toEqual({ view: 'agents' });
    expect(
      lobbySearchWithRoomPanel(
        { agent: 'agent-01', layout: 'conversation', message: '$message', view: 'resources' },
        'agents',
      ),
    ).toEqual({ layout: 'conversation', view: 'agents' });
  });
  it('在那一页上点一行看详情，页面留着；关掉详情也留着', () => {
    const search = { layout: 'conversation', view: 'agents' } as const;
    expect(lobbySearchWithAgent(search, 'agent-01', 'conversation')).toEqual({
      agent: 'agent-01',
      ...search,
    });
    expect(lobbySearchWithAgent({ agent: 'agent-01', ...search }, null, 'conversation')).toEqual(
      search,
    );
  });
  it('回对话、打开私聊、去看某条消息都离开那一页', () => {
    const search = { agent: 'agent-01', layout: 'conversation', view: 'agents' } as const;
    expect(lobbySearchWithRoomPanel(search, 'conversation')).toEqual({
      layout: 'conversation',
      view: 'conversation',
    });
    expect(lobbySearchWithDirectSession(search, 'private-room')).toEqual({
      direct: 'private-room',
      layout: 'conversation',
    });
    expect(lobbySearchWithMessage(search, '$message')).toEqual({
      layout: 'conversation',
      message: '$message',
    });
    // 放开正在看的消息时哪一页都不动。
    expect(lobbySearchWithMessage(search, null)).toEqual(search);
  });
  it('换到场景时那一页收起来：场景里没有它', () => {
    expect(lobbySearchWithLayout({ layout: 'conversation', view: 'agents' }, 'scene')).toEqual({
      layout: 'scene',
    });
  });
});

describe('房间页版面路由', () => {
  it('地址里只认对话和场景两种版面', () => {
    expect(normalizeLobbySearch({ layout: 'conversation' })).toEqual({ layout: 'conversation' });
    expect(normalizeLobbySearch({ layout: 'scene', view: 'resources' })).toEqual({
      layout: 'scene',
      view: 'resources',
    });
    expect(normalizeLobbySearch({ layout: 'table' })).toEqual({});
    expect(normalizeLobbySearch({ layout: 1 })).toEqual({});
  });
  it('换选择、换视图时版面留在地址里', () => {
    const search = { directory: 'open', layout: 'conversation' } as const;
    expect(lobbySearchWithDirectSession(search, 'private-room')).toEqual({
      direct: 'private-room',
      directory: 'open',
      layout: 'conversation',
    });
    expect(lobbySearchWithMessage(search, '$message')).toEqual({
      directory: 'open',
      layout: 'conversation',
      message: '$message',
    });
    expect(lobbySearchWithView({ ...search, view: 'resources' }, 'space')).toEqual(search);
    expect(lobbySearchWithRoomPanel({ ...search, direct: 'private-room' }, 'resources')).toEqual({
      directory: 'open',
      layout: 'conversation',
      view: 'resources',
    });
  });
  it('对话版面里选中 Agent，对话、私聊和正在看的消息都留着', () => {
    const search = {
      direct: 'private-room',
      layout: 'conversation',
      message: '$message',
      view: 'conversation',
    } as const;
    expect(lobbySearchWithAgent(search, 'agent-01', 'conversation')).toEqual({
      agent: 'agent-01',
      ...search,
    });
    // 场景里选中 Agent 照旧收起对话面板。
    expect(lobbySearchWithAgent({ ...search, layout: 'scene' }, 'agent-01', 'scene')).toEqual({
      agent: 'agent-01',
      layout: 'scene',
    });
  });
  it('换到场景回到画面本身，换到对话什么都不丢', () => {
    const search = {
      agent: 'agent-01',
      direct: 'private-room',
      directory: 'open',
      layout: 'conversation',
      message: '$message',
      view: 'conversation',
    } as const;
    expect(lobbySearchWithLayout(search, 'scene')).toEqual({
      agent: 'agent-01',
      directory: 'open',
      layout: 'scene',
    });
    expect(
      lobbySearchWithLayout({ view: 'resources', message: '$message' }, 'conversation'),
    ).toEqual({
      layout: 'conversation',
      message: '$message',
      view: 'resources',
    });
  });
});
