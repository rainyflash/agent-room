import type { AgentLifecycle } from '@agent-room/protocol';

import type { LobbyAgent, LobbyRoom } from '@/features/lobby/domain/lobby';
import type { RoomHuman } from '@/features/lobby/domain/room-participants';
import { speechBubbleText, type RoomSpeech } from '@/features/lobby/domain/room-speech';
import type { LobbySceneProjection } from '@/features/lobby/domain/scene-projection';
import type { PublicWatch } from './public-watch';

/** 说过的话在人物头上留这么久（按服务器时间算）：快照 3 秒一换、网页 5 秒一问，比房间里留得久些。 */
export const watchSpeechLifetimeMs = 30_000;
/** 场景里同时最多冒这么多个气泡。 */
const speechBubbles = 3;
/** 服务端只列在线的 Agent，在线状态已经按租约算好；这里只是给场景一个足够长的租约。 */
const sceneLeaseMs = 300_000;
const online: AgentLifecycle = Object.freeze({
  archived: false,
  archiveReason: null,
  connection: 'online',
  offlineSinceUnixMs: null,
  reception: 'unknown',
});

/**
 * 把快照摆成大厅场景用的房间：只有在线的 Agent 站在场景里。编号就是人物的身份，场景、名单和
 * 气泡都只拿它对上号，不是 Matrix 的 ID。
 */
export function watchRoom(watch: PublicWatch): LobbyRoom {
  return Object.freeze({
    agents: Object.freeze(
      watch.participants
        .filter((participant) => participant.kind !== 'person' && participant.online)
        .map((participant): LobbyAgent =>
          Object.freeze({
            agentId: participant.key,
            displayName: participant.name,
            // 快照不说有几个实例；在线就至少有一个，名单上别写成“0 个实例”。
            instanceIds: [participant.key],
            lifecycle: online,
            matrixUserId: participant.key,
            status: participant.status ?? 'idle',
            statusExpiresAtUnixMs: watch.updatedAtUnixMs + sceneLeaseMs,
            // 服务端只给验过实例签名的在线状态。
            trust: 'verified',
            visibility: 'coarse',
          }),
        ),
    ),
    name: watch.lobby.name,
    observedAtUnixMs: watch.updatedAtUnixMs,
    roomId: `watch:${watch.lobby.slug}`,
  });
}

/** 最近说过话的人，按编号排好，场景里的位置才稳定。 */
export function watchHumans(watch: PublicWatch): readonly RoomHuman[] {
  return Object.freeze(
    watch.participants
      .filter((participant) => participant.kind === 'person')
      .map((participant) =>
        Object.freeze({
          displayName: participant.name,
          isSelf: false,
          matrixUserId: participant.key,
        }),
      )
      .toSorted((a, b) => a.matrixUserId.localeCompare(b.matrixUserId)),
  );
}

/** 最近半分钟里说过话的人物头上冒气泡，每人一个、新的优先，最多三个。 */
export function watchSpeech(
  scene: LobbySceneProjection,
  watch: PublicWatch,
): readonly RoomSpeech[] {
  const result: RoomSpeech[] = [];
  const speakers = new Set<string>();
  for (const message of watch.messages.toReversed()) {
    if (message.sentAtUnixMs + watchSpeechLifetimeMs <= watch.updatedAtUnixMs) break;
    // 不公开的不冒；只有标记符号、去掉后没字的也不冒。
    const text = message.withheld ? '' : speechBubbleText(message.text);
    if (text === '') continue;
    const agent = scene.nodes.find((node) => node.agentId === message.author);
    const human =
      agent === undefined
        ? scene.humans?.find((node) => node.matrixUserId === message.author)
        : undefined;
    const characterId = agent?.agentId ?? human?.characterId;
    const name = agent?.displayName ?? human?.displayName;
    if (characterId === undefined || name === undefined || speakers.has(characterId)) continue;
    speakers.add(characterId);
    result.push({ characterId, messageId: message.key, name, text });
    if (result.length === speechBubbles) break;
  }
  return result;
}
