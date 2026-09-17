import type { MatrixClient, MatrixEvent } from 'matrix-js-sdk';
import { DIRECTION_BACKWARD, DIRECTION_FORWARD } from '@/shared/matrix/matrix-sdk-enums';
import { err, ok, type Result } from '@/shared/result';

import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';

export const matrixMessagePreviewEventType = 'io.github.rainyflash.agentroom.message.preview.v1';
export const matrixMessageRevisionEventType = 'io.github.rainyflash.agentroom.message.revision.v1';
export const matrixMessagePreviewEventTypeV2 = 'io.github.rainyflash.agentroom.message.preview.v2';
export const matrixMessageRevisionEventTypeV2 =
  'io.github.rainyflash.agentroom.message.revision.v2';
export const matrixModerationNoticeEventType =
  'io.github.rainyflash.agentroom.moderation.notice.v1';
export const matrixAgentRoomEventNamespace = 'io.github.rainyflash.agentroom';
const legacyMatrixAgentRoomEventNamespace = ['org', 'agentroom'].join('.');

const projectedTimelineEventTypes = new Set([
  matrixMessagePreviewEventType,
  matrixMessagePreviewEventTypeV2,
  matrixMessageRevisionEventType,
  matrixMessageRevisionEventTypeV2,
  matrixModerationNoticeEventType,
]);
const historyPageSize = 200;
const historyLimit = 5000;

export type MatrixMessageTimelineEvent = {
  readonly content: unknown;
  readonly endToEndEncrypted: boolean;
  readonly eventId: string | undefined;
  readonly sender: string | undefined;
  readonly serverTimestamp: number;
  readonly type: string;
};

export type MatrixMessageRoomSnapshot = {
  readonly windowSize?: number;
  readonly hasOlder?: boolean;
  readonly roomId: string;
  readonly timelineEvents: readonly MatrixMessageTimelineEvent[];
};

export type MatrixMessageSourceRead =
  | { readonly kind: 'matrix-unavailable' }
  | { readonly kind: 'room-not-joined' }
  | { readonly kind: 'ready'; readonly room: MatrixMessageRoomSnapshot };

export type MatrixMessageSource = {
  loadOlder?(
    roomId: string,
  ): Promise<Result<void, { readonly code: string; readonly retryable: boolean }>>;
  read(roomId: string): MatrixMessageSourceRead;
  subscribe(roomId: string, listener: () => void): () => void;
};

export class MatrixSdkMessageSource implements MatrixMessageSource {
  readonly #clients: MatrixClientSource;
  #client: MatrixClient | null = null;
  readonly #windows = new Map<string, number>();
  readonly #loading = new Map<
    string,
    Promise<Result<void, { readonly code: string; readonly retryable: boolean }>>
  >();
  readonly #listeners = new Map<string, Set<() => void>>();

  constructor(clients: MatrixClientSource) {
    this.#clients = clients;
  }

  read(roomId: string): MatrixMessageSourceRead {
    const client = this.#current();
    if (client === null) {
      return { kind: 'matrix-unavailable' };
    }
    const room = client.getRoom(roomId);
    if (room?.getMyMembership() !== 'join') {
      return { kind: 'room-not-joined' };
    }
    const state = room.getLiveTimeline().getState(DIRECTION_FORWARD);
    if (state === undefined) {
      return { kind: 'room-not-joined' };
    }

    return {
      kind: 'ready',
      room: Object.freeze({
        roomId,
        windowSize: this.#windows.get(roomId) ?? historyPageSize,
        hasOlder: room.getLiveTimeline().getPaginationToken(DIRECTION_BACKWARD) !== null,
        timelineEvents: Object.freeze([
          ...room
            .getLiveTimeline()
            .getEvents()
            .filter(isProjectedTimelineEvent)
            .map(toTimelineEvent),
          ...state.getStateEvents(matrixModerationNoticeEventType).map(toTimelineEvent),
        ]),
      }),
    };
  }

  subscribe(roomId: string, listener: () => void): () => void {
    const listeners = this.#listeners.get(roomId) ?? new Set<() => void>();
    listeners.add(listener);
    this.#listeners.set(roomId, listeners);
    const detach = this.#clients.subscribe(listener);
    return () => {
      detach();
      listeners.delete(listener);
      if (listeners.size === 0) this.#listeners.delete(roomId);
    };
  }

  loadOlder(
    roomId: string,
  ): Promise<Result<void, { readonly code: string; readonly retryable: boolean }>> {
    this.#current();
    const pending = this.#loading.get(roomId);
    if (pending) return pending;
    const operation = this.#load(roomId).catch(() =>
      err({ code: 'history.load_failed', retryable: true }),
    );
    this.#loading.set(roomId, operation);
    void operation.finally(() => {
      if (this.#loading.get(roomId) === operation) this.#loading.delete(roomId);
    });
    return operation;
  }

  async #load(
    roomId: string,
  ): Promise<Result<void, { readonly code: string; readonly retryable: boolean }>> {
    const client = this.#current();
    const room = client?.getRoom(roomId);
    if (!client || room?.getMyMembership() !== 'join')
      return err({ code: 'history.room_unavailable', retryable: true });
    const currentWindow = this.#windows.get(roomId) ?? historyPageSize;
    if (currentWindow >= historyLimit) return err({ code: 'history.limit', retryable: false });
    const timeline = room.getLiveTimeline();
    const loadedPreviews = timeline
      .getEvents()
      .filter(
        (event) =>
          event.getType() === matrixMessagePreviewEventType ||
          event.getType() === matrixMessagePreviewEventTypeV2,
      ).length;
    try {
      if (
        loadedPreviews <= currentWindow &&
        timeline.getPaginationToken(DIRECTION_BACKWARD) !== null
      )
        await client.scrollback(room, historyPageSize);
      if (this.#current() !== client || room.getMyMembership() !== 'join')
        return err({ code: 'history.session_changed', retryable: true });
      const available = room
        .getLiveTimeline()
        .getEvents()
        .filter(
          (event) =>
            event.getType() === matrixMessagePreviewEventType ||
            event.getType() === matrixMessagePreviewEventTypeV2,
        ).length;
      this.#windows.set(
        roomId,
        available > currentWindow
          ? Math.min(currentWindow + historyPageSize, historyLimit)
          : currentWindow,
      );
      for (const listener of this.#listeners.get(roomId) ?? []) listener();
      return ok(undefined);
    } catch {
      return err({ code: 'history.load_failed', retryable: true });
    }
  }

  #current(): MatrixClient | null {
    const client = this.#clients.current();
    if (client !== this.#client) {
      this.#client = client;
      this.#windows.clear();
      this.#loading.clear();
    }
    return client;
  }
}

function isProjectedTimelineEvent(event: MatrixEvent): boolean {
  const eventType = event.getType();
  return (
    projectedTimelineEventTypes.has(eventType) ||
    isNamespacedEvent(eventType, matrixAgentRoomEventNamespace) ||
    isNamespacedEvent(eventType, legacyMatrixAgentRoomEventNamespace)
  );
}

function isNamespacedEvent(eventType: string, namespace: string): boolean {
  return eventType.startsWith(`${namespace}.`);
}

function toTimelineEvent(event: MatrixEvent): MatrixMessageTimelineEvent {
  return Object.freeze({
    content: event.getContent(),
    endToEndEncrypted: event.isEncrypted(),
    eventId: event.getId(),
    sender: event.getSender(),
    serverTimestamp: event.getTs(),
    type: event.getType(),
  });
}
