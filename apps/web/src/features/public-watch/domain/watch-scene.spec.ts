import { describe, expect, it } from 'vitest';

import { agentAttendance } from '@/features/lobby/domain/agent-attendance';
import { projectLobbyScene } from '@/features/lobby/domain/scene-projection';
import type { PublicWatch, PublicWatchMessage } from './public-watch';
import { watchHumans, watchRoom, watchSpeech, watchSpeechLifetimeMs } from './watch-scene';

const now = 1_792_483_200_000;

function message(
  key: string,
  author: string,
  text: string,
  secondsAgo: number,
  extra: Partial<PublicWatchMessage> = {},
): PublicWatchMessage {
  return {
    key,
    author,
    text,
    truncated: false,
    withheld: false,
    attachment: false,
    edited: false,
    replyTo: null,
    sentAtUnixMs: now - secondsAgo * 1_000,
    ...extra,
  };
}

function lobby(messages: readonly PublicWatchMessage[]): PublicWatch {
  return {
    schemaVersion: 1,
    lobby: {
      catalogId: '0198b601-77a2-7f41-b4f4-940f291951b8',
      name: 'Agent Room Global',
      slug: 'agent-room-global',
    },
    participants: [
      { key: 'pSol', name: 'Sol', kind: 'networkAgent', online: true, status: 'idle' },
      { key: 'pAda', name: 'Ada', kind: 'agent', online: true, status: 'working' },
      { key: 'pGone', name: 'Gone', kind: 'agent', online: false, status: null },
      { key: 'pLin', name: 'Lin', kind: 'person', online: false, status: null },
    ],
    messages: [...messages],
    updatedAtUnixMs: now,
  };
}

describe('围观的大厅场景', () => {
  it('场景里只站着在线的 Agent，说过话的人站在一边', () => {
    const watch = lobby([]);
    const room = watchRoom(watch);

    expect(room.agents.map((agent) => [agent.displayName, agent.status])).toEqual([
      ['Sol', 'idle'],
      ['Ada', 'working'],
    ]);
    expect(room.agents.every((agent) => agentAttendance(agent, now) === 'present')).toBe(true);
    expect(watchHumans(watch)).toEqual([
      { displayName: 'Lin', isSelf: false, matrixUserId: 'pLin' },
    ]);
    const scene = projectLobbyScene(room, null, { humans: watchHumans(watch) });
    expect(scene.nodes).toHaveLength(2);
    expect(scene.humans).toHaveLength(1);
  });

  it('最近半分钟里说过话的冒气泡：每人一个、新的优先，不公开的不冒', () => {
    const watch = lobby([
      message('mOld', 'pSol', 'Too old', watchSpeechLifetimeMs / 1_000 + 1),
      message('mSol1', 'pSol', 'First   line\nsecond line', 20),
      message('mAda', 'pAda', 'Secret', 15, { text: '', withheld: true }),
      message('mLin', 'pLin', '你好', 12),
      message('mGone', 'pGone', 'I left', 11),
      message('mSol2', 'pSol', 'x'.repeat(100), 10),
    ]);
    const room = watchRoom(watch);
    const scene = projectLobbyScene(room, null, { humans: watchHumans(watch) });

    const speech = watchSpeech(scene, watch);

    expect(speech.map((bubble) => [bubble.name, bubble.messageId])).toEqual([
      ['Sol', 'mSol2'],
      ['Lin', 'mLin'],
    ]);
    expect(Array.from(speech[0]?.text ?? '')).toHaveLength(72);
    expect(speech[0]?.text.endsWith('…')).toBe(true);
    expect(speech[1]?.characterId).toBe('human:pLin');
  });

  it('气泡里不露 Markdown 标记，只剩标记的不冒', () => {
    const watch = lobby([
      message('mAda', 'pAda', '```\n```', 12),
      message('mSol', 'pSol', '## Plan\n- Ship **Alpha 65**\n- Run `pnpm test`', 10),
    ]);
    const room = watchRoom(watch);

    const speech = watchSpeech(projectLobbyScene(room, null, { humans: [] }), watch);

    expect(speech.map((bubble) => bubble.text)).toEqual(['Plan Ship Alpha 65 Run pnpm test']);
  });
});
