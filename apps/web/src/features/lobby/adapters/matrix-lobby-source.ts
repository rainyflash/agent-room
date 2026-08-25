import type { Direction, RoomState } from 'matrix-js-sdk';

import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';

export const matrixAgentStatusEventType = 'io.github.rainyflash.agentroom.agent.status.v1';
const forwardTimelineDirection = 'f' as Direction;

export type MatrixLobbyStateEvent = {
  readonly content: unknown;
  readonly sender: string | undefined;
  readonly stateKey: string | undefined;
};

export type MatrixLobbyRoomSnapshot = {
  readonly joinedMemberIds: readonly string[];
  readonly name: string;
  readonly roomId: string;
  readonly statusEvents: readonly MatrixLobbyStateEvent[];
  readonly topic?: string;
};

export type MatrixLobbySourceRead =
  | { readonly kind: 'matrix-unavailable' }
  | { readonly kind: 'room-not-joined' }
  | { readonly kind: 'ready'; readonly room: MatrixLobbyRoomSnapshot };

export type MatrixLobbySource = {
  read(roomId: string): MatrixLobbySourceRead;
  subscribe(roomId: string, listener: () => void): () => void;
};

export class MatrixSdkLobbySource implements MatrixLobbySource {
  readonly #clients: MatrixClientSource;

  constructor(clients: MatrixClientSource) {
    this.#clients = clients;
  }

  read(roomId: string): MatrixLobbySourceRead {
    const client = this.#clients.current();
    if (client === null) {
      return { kind: 'matrix-unavailable' };
    }
    const room = client.getRoom(roomId);
    if (room === null) {
      return { kind: 'room-not-joined' };
    }
    const state = room.getLiveTimeline().getState(forwardTimelineDirection);
    if (state === undefined) {
      return { kind: 'room-not-joined' };
    }
    const topic = readRoomTopic(state);
    return {
      kind: 'ready',
      room: Object.freeze({
        joinedMemberIds: Object.freeze(
          room
            .getJoinedMembers()
            .map((member) => member.userId)
            .toSorted(),
        ),
        name: room.name.trim() || roomId,
        roomId,
        statusEvents: Object.freeze(
          state.getStateEvents(matrixAgentStatusEventType).map((event) =>
            Object.freeze({
              content: event.getContent(),
              sender: event.getSender(),
              stateKey: event.getStateKey(),
            }),
          ),
        ),
        ...(topic === undefined ? {} : { topic }),
      }),
    };
  }

  subscribe(roomId: string, listener: () => void): () => void {
    void roomId;
    return this.#clients.subscribe(listener);
  }
}

function readRoomTopic(state: RoomState): string | undefined {
  const topicEvent = state.getStateEvents('m.room.topic', '');
  if (topicEvent === null) {
    return undefined;
  }
  const content: unknown = topicEvent.getContent();
  if (typeof content !== 'object' || content === null || !('topic' in content)) {
    return undefined;
  }
  const topic = content.topic;
  return typeof topic === 'string' && topic.trim().length > 0 ? topic.trim() : undefined;
}
