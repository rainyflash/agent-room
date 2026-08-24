import type { MatrixEvent } from 'matrix-js-sdk';

import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';

export const matrixMessagePreviewEventType = 'org.agentroom.message.preview.v1';
export const matrixMessageRevisionEventType = 'org.agentroom.message.revision.v1';

const projectedTimelineEventTypes = new Set([
  matrixMessagePreviewEventType,
  matrixMessageRevisionEventType,
]);

export type MatrixMessageTimelineEvent = {
  readonly content: unknown;
  readonly eventId: string | undefined;
  readonly sender: string | undefined;
  readonly serverTimestamp: number;
  readonly type: string;
};

export type MatrixMessageRoomSnapshot = {
  readonly roomId: string;
  readonly timelineEvents: readonly MatrixMessageTimelineEvent[];
};

export type MatrixMessageSourceRead =
  | { readonly kind: 'matrix-unavailable' }
  | { readonly kind: 'room-not-joined' }
  | { readonly kind: 'ready'; readonly room: MatrixMessageRoomSnapshot };

export type MatrixMessageSource = {
  read(roomId: string): MatrixMessageSourceRead;
  subscribe(roomId: string, listener: () => void): () => void;
};

export class MatrixSdkMessageSource implements MatrixMessageSource {
  readonly #clients: MatrixClientSource;

  constructor(clients: MatrixClientSource) {
    this.#clients = clients;
  }

  read(roomId: string): MatrixMessageSourceRead {
    const client = this.#clients.current();
    if (client === null) {
      return { kind: 'matrix-unavailable' };
    }
    const room = client.getRoom(roomId);
    if (room === null) {
      return { kind: 'room-not-joined' };
    }

    return {
      kind: 'ready',
      room: Object.freeze({
        roomId,
        timelineEvents: Object.freeze(
          room.getLiveTimeline().getEvents().filter(isProjectedTimelineEvent).map(toTimelineEvent),
        ),
      }),
    };
  }

  subscribe(roomId: string, listener: () => void): () => void {
    void roomId;
    return this.#clients.subscribe(listener);
  }
}

function isProjectedTimelineEvent(event: MatrixEvent): boolean {
  return projectedTimelineEventTypes.has(event.getType());
}

function toTimelineEvent(event: MatrixEvent): MatrixMessageTimelineEvent {
  return Object.freeze({
    content: event.getContent(),
    eventId: event.getId(),
    sender: event.getSender(),
    serverTimestamp: event.getTs(),
    type: event.getType(),
  });
}
