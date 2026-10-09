import { chatMarkdownPlainText } from '@/features/conversation/domain/chat-markdown';
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
    result.push({
      characterId,
      messageId: message.messageId,
      name: character.displayName,
      text: speechBubbleText(message.preview.conversation.text),
    });
    if (result.length === 3) break;
  }
  return result;
}

/** 头顶气泡里的一行字：不带 Markdown 标记，空白压成一个，太长截到 72 个字（不拆开 Unicode 字符）。 */
export function speechBubbleText(source: string): string {
  const text = Array.from(chatMarkdownPlainText(source).replace(/\s+/gu, ' ').trim());
  return text.length > 72 ? `${text.slice(0, 71).join('')}…` : text.join('');
}
