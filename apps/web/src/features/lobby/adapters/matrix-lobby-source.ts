import type { MatrixPresenceObservation } from '@agent-room/protocol';
import type { Direction, RoomState } from 'matrix-js-sdk';

import type { AgentPresenceSource } from './matrix-presence-tracker';
import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';

export const matrixAgentStatusEventType = 'io.github.rainyflash.agentroom.agent.status.v1';
export const matrixRosterPolicyEventType = 'io.github.rainyflash.agentroom.roster.policy.v1';
const forwardTimelineDirection = 'f' as Direction;

export type MatrixLobbyStateEvent = {
  readonly content: unknown;
  readonly sender: string | undefined;
  readonly stateKey: string | undefined;
};

export type MatrixLobbyMemberName = {
  readonly displayName: string;
  readonly matrixUserId: string;
};

export type MatrixLobbyRoomSnapshot = {
  /**
   * 这台设备上登录的人此刻能不能在这里发消息，按 Matrix 的发言门槛和他的级别算：房间归档、他被禁言
   * 或只能查看时都不能。
   */
  readonly canSpeak: boolean;
  readonly encrypted: boolean;
  readonly joinedMemberIds: readonly string[];
  /**
   * 已加入的成员在 Matrix 上的显示名，按用户 ID 排好；没设名字的，SDK 给的是用户 ID。
   * 只有加密的房间（私人房间）读：公开大厅不给成员名字，人多时也省得每次同步都抄一遍。
   */
  readonly memberNames?: readonly MatrixLobbyMemberName[];
  readonly name: string;
  readonly roomId: string;
  readonly statusEvents: readonly MatrixLobbyStateEvent[];
  readonly rosterPolicy?: unknown;
  readonly topic?: string;
  /**
   * 房间里写名片（`liveness: "presence"`）的 Agent 此刻的 Matrix 在线状态，按 Matrix 用户 ID。
   * 还没拿到的不在里面。
   */
  readonly presence?: ReadonlyMap<string, MatrixPresenceObservation>;
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
  readonly #presence: AgentPresenceSource | undefined;

  /** 不给 `presence` 就不读在线状态，写名片的 Agent 都算离线。 */
  constructor(clients: MatrixClientSource, presence?: AgentPresenceSource) {
    this.#clients = clients;
    this.#presence = presence;
  }

  read(roomId: string): MatrixLobbySourceRead {
    const client = this.#clients.current();
    if (client === null) {
      return { kind: 'matrix-unavailable' };
    }
    const room = client.getRoom(roomId);
    if (room?.getMyMembership() !== 'join') {
      return { kind: 'room-not-joined' };
    }
    const state = room.getLiveTimeline().getState(forwardTimelineDirection);
    if (state === undefined) {
      return { kind: 'room-not-joined' };
    }
    const topic = readRoomTopic(state);
    const rosterPolicy = state.getStateEvents(matrixRosterPolicyEventType, '');
    const encrypted = state.getStateEvents('m.room.encryption', '') !== null;
    // 按用户 ID 的字面排好：同步时成员的先后会变，排好了没变的快照才认得出没变。
    const joinedMembers = room
      .getJoinedMembers()
      .toSorted((left, right) =>
        left.userId < right.userId ? -1 : left.userId > right.userId ? 1 : 0,
      );
    const joinedMemberIds = joinedMembers.map((member) => member.userId);
    const statusEvents = state.getStateEvents(matrixAgentStatusEventType).map((event) =>
      Object.freeze({
        content: event.getContent(),
        sender: event.getSender(),
        stateKey: event.getStateKey(),
      }),
    );
    const presence = this.#cardPresence(statusEvents, joinedMemberIds);
    const self = client.getUserId();
    return {
      kind: 'ready',
      room: Object.freeze({
        canSpeak: self !== null && state.maySendMessage(self),
        encrypted,
        joinedMemberIds: Object.freeze(joinedMemberIds),
        ...(encrypted
          ? {
              memberNames: Object.freeze(
                joinedMembers.map((member) =>
                  Object.freeze({
                    displayName: member.rawDisplayName,
                    matrixUserId: member.userId,
                  }),
                ),
              ),
            }
          : {}),
        name: room.name.trim() || roomId,
        roomId,
        ...(rosterPolicy === null ? {} : { rosterPolicy: rosterPolicy.getContent() }),
        statusEvents: Object.freeze(statusEvents),
        ...(topic === undefined ? {} : { topic }),
        ...(presence === undefined ? {} : { presence }),
      }),
    };
  }

  subscribe(roomId: string, listener: () => void): () => void {
    void roomId;
    const detachClients = this.#clients.subscribe(listener);
    // 问到的在线状态不跟同步一起来，单独通知。
    const detachPresence = this.#presence?.subscribe(listener);
    return () => {
      detachClients();
      detachPresence?.();
    };
  }

  /**
   * 只看还在房间里、写名片的 Agent：离开的人名片还留在房间状态里，问不到也用不着。
   * 是不是名片在这里只看 `liveness` 一个字段，整条校验在网关里。
   */
  #cardPresence(
    statusEvents: readonly MatrixLobbyStateEvent[],
    joinedMemberIds: readonly string[],
  ): ReadonlyMap<string, MatrixPresenceObservation> | undefined {
    if (this.#presence === undefined) return undefined;
    const joined = new Set(joinedMemberIds);
    const cardHolders = new Set(
      statusEvents.flatMap((event) =>
        event.sender !== undefined && joined.has(event.sender) && isCard(event.content)
          ? [event.sender]
          : [],
      ),
    );
    return cardHolders.size === 0 ? undefined : this.#presence.observe([...cardHolders]);
  }
}

function isCard(content: unknown): boolean {
  return (
    typeof content === 'object' &&
    content !== null &&
    'liveness' in content &&
    content.liveness === 'presence'
  );
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
