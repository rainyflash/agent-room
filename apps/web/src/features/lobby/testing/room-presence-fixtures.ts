import type { LobbyAgent, LobbyRoom } from '../domain/lobby';
import type { MessageActor, RoomMessageSignal } from '@/features/messages/domain/message';

export const presenceTime = 1_780_000_000_000;
export const selfIdentity = { matrixUserId: '@self:room.test', displayName: '小林' };
export const guestActor: MessageActor = {
  kind: 'human',
  provenance: 'human',
  matrixUserId: '@guest:room.test',
  displayName: '访客',
  principalId: 'guest',
};
export function presenceAgent(index: number): LobbyAgent {
  return {
    agentId: `agent-${String(index)}`,
    matrixUserId: `@agent-${String(index)}:room.test`,
    displayName: `助手 ${String(index)}`,
    instanceIds: [`instance-${String(index)}`],
    status: 'idle',
    statusExpiresAtUnixMs: presenceTime + 60_000,
    trust: 'verified',
    visibility: 'coarse',
  };
}
export function presenceRoom(count = 24): LobbyRoom {
  const agents = Array.from({ length: count }, (_, index) => presenceAgent(index));
  return {
    roomId: '!lobby:room.test',
    name: '测试大厅',
    observedAtUnixMs: presenceTime,
    agents,
    joinedMemberIds: [
      selfIdentity.matrixUserId,
      guestActor.matrixUserId,
      ...agents.map((agent) => agent.matrixUserId),
    ],
  };
}
export function presenceMessage(
  id: string,
  overrides: Partial<RoomMessageSignal> = {},
): RoomMessageSignal {
  return {
    actor: guestActor,
    content: null,
    edited: false,
    endToEndEncrypted: false,
    lifecycle: 'active',
    matrixEventId: `$${id}`,
    messageId: id,
    roomId: '!lobby:room.test',
    serverTimestamp: presenceTime,
    signatureStatus: 'matrix_sender_matched',
    preview: {
      conversation: { text: `你好，${id}`, mentions: [] },
      contentType: 'text/plain',
      riskFlags: [],
      sensitivity: 'normal',
      title: id,
      summary: id,
    },
    ...overrides,
  };
}
export function presenceAgentActor(index: number): MessageActor {
  const agent = presenceAgent(index);
  return {
    kind: 'agent',
    provenance: 'human_confirmed_agent',
    agentId: agent.agentId,
    matrixUserId: agent.matrixUserId,
    displayName: agent.displayName,
    instanceId: `instance-${String(index)}`,
  };
}
