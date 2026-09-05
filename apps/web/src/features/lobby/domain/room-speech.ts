import type { RoomMessageSignal } from '@/features/messages/domain/message';
import type { LobbySceneProjection } from './scene-projection';

export type RoomSpeech = {
  readonly characterId: string;
  readonly messageId: string;
  readonly name: string;
  readonly text: string;
};

export function projectRoomSpeech(
  scene: LobbySceneProjection,
  messages: readonly RoomMessageSignal[],
): readonly RoomSpeech[] {
  const result: RoomSpeech[] = [];
  const speakers = new Set<string>();
  for (const message of messages.toSorted((a, b) => b.serverTimestamp - a.serverTimestamp)) {
    if (
      message.roomId !== scene.roomId ||
      message.lifecycle !== 'active' ||
      message.preview?.conversation === undefined
    )
      continue;
    const actor = message.actor;
    const character =
      actor.kind === 'agent'
        ? scene.nodes.find(
            (node) => node.agentId === actor.agentId && node.matrixUserId === actor.matrixUserId,
          )
        : scene.humans?.find((node) => node.matrixUserId === actor.matrixUserId);
    if (character === undefined) continue;
    const characterId = 'agentId' in character ? character.agentId : character.characterId;
    if (speakers.has(characterId)) continue;
    speakers.add(characterId);
    const text = Array.from(message.preview.conversation.text.replace(/\s+/gu, ' ').trim());
    result.push({
      characterId,
      messageId: message.messageId,
      name: character.displayName,
      text: text.length > 72 ? `${text.slice(0, 71).join('')}…` : text.join(''),
    });
    if (result.length === 3) break;
  }
  return result;
}
