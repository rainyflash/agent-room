import { describe, expect, it } from 'vitest';
import { projectLobbyScene } from './scene-projection';
import { isWalkableFloor } from './room-floor';
import { roomHumans } from './room-participants';
import { projectRoomSpeech } from './room-speech';
import {
  guestActor,
  presenceAgent,
  presenceAgentActor,
  presenceMessage,
  presenceRoom,
  selfIdentity,
} from '../testing/room-presence-fixtures';

describe('房间人物位置与身份', () => {
  it('成员加入、离开以及状态变化都保留仍在场人物的位置', () => {
    const room = presenceRoom();
    const humans = roomHumans(room, [], selfIdentity);
    const initial = projectLobbyScene(room, null, { humans });
    const changed = {
      ...room,
      agents: [
        ...room.agents.slice(1).map((agent) => ({ ...agent, status: 'working' as const })),
        presenceAgent(24),
      ],
    };
    const next = projectLobbyScene(changed, null, { previous: initial.layout, humans });
    for (const node of initial.nodes.slice(1))
      expect(next.layout.get(node.agentId)).toEqual(initial.layout.get(node.agentId));
    expect(next.layout.get('human:@self:room.test')).toEqual(
      initial.layout.get('human:@self:room.test'),
    );
    expect(next.layout.has('agent-0')).toBe(false);
    expect(next.nodes).toHaveLength(24);
  });

  it('200 个 Agent 与人类共处时不丢人物、不复用站位、不进入家具', () => {
    const room = presenceRoom(200);
    const projection = projectLobbyScene(room, null, {
      humans: roomHumans(room, [presenceMessage('hello')], selfIdentity),
    });
    const places = [...projection.layout.values()];
    expect(projection.nodes).toHaveLength(200);
    expect(projection.humans).toHaveLength(2);
    expect(places).toHaveLength(202);
    expect(new Set(places.map((place) => place.slot)).size).toBe(202);
    expect(places.every((place) => isWalkableFloor(place, 22))).toBe(true);
  });

  it('只有已加入房间的已知人类与当前身份生成人物，私聊和 Agent 不混入', () => {
    const room = presenceRoom();
    const messages = [
      presenceMessage('private', { roomId: '!private:room.test' }),
      presenceMessage('agent', { actor: presenceAgentActor(0) }),
      presenceMessage('impostor', {
        actor: { ...guestActor, matrixUserId: presenceAgent(0).matrixUserId },
      }),
      presenceMessage('left', { actor: { ...guestActor, matrixUserId: '@left:room.test' } }),
    ];
    expect(roomHumans(room, messages, selfIdentity)).toEqual([{ ...selfIdentity, isSelf: true }]);
    expect(roomHumans(room, [...messages, presenceMessage('public')], selfIdentity)).toHaveLength(
      2,
    );
    expect(
      roomHumans({ ...room, joinedMemberIds: [] }, [presenceMessage('public')], selfIdentity),
    ).toEqual([]);
  });
});

describe('房间公开发言气泡', () => {
  const room = presenceRoom();
  const scene = projectLobbyScene(room, null, {
    humans: roomHumans(room, [presenceMessage('hello')], selfIdentity),
  });

  it('绑定人物及 Matrix 身份，过滤其他房间、撤回内容和冒名消息', () => {
    const messages = [
      presenceMessage('public', { actor: presenceAgentActor(0) }),
      presenceMessage('private', { roomId: '!private:room.test', actor: presenceAgentActor(1) }),
      presenceMessage('redacted', { lifecycle: 'redacted', actor: presenceAgentActor(2) }),
      presenceMessage('impostor', {
        actor: { ...presenceAgentActor(3), matrixUserId: '@fake:room.test' },
      }),
      presenceMessage('resource', { preview: null }),
    ];
    expect(projectRoomSpeech(scene, messages).map((bubble) => bubble.messageId)).toEqual([
      'public',
    ]);
  });

  it('每人只显示最新发言、最多三人，长文本不会拆开 Unicode 字符', () => {
    const message = presenceMessage('long');
    if (message.preview === null) throw new Error('测试消息缺少预览');
    const messages = [
      presenceMessage('old', { serverTimestamp: 1 }),
      {
        ...message,
        preview: { ...message.preview, conversation: { text: '🙂'.repeat(80), mentions: [] } },
      },
      ...[0, 1, 2].map((index) =>
        presenceMessage(`agent-${String(index)}`, {
          actor: presenceAgentActor(index),
          serverTimestamp: 2,
        }),
      ),
    ];
    const bubbles = projectRoomSpeech(scene, messages);
    expect(bubbles).toHaveLength(3);
    expect(bubbles[0]?.text).toBe(`${'🙂'.repeat(71)}…`);
    expect(new Set(bubbles.map((bubble) => bubble.characterId)).size).toBe(3);
  });
});
