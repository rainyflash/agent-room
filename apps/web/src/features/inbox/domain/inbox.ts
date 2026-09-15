import type { MessageGateway, RoomMessageSignal } from '@/features/messages/domain/message';
import {
  compareCursors,
  type WorkspaceIndex,
} from '@/features/personal-workspace/domain/workspace-document';
import type { Result } from '@/shared/result';

export type InboxRoom = {
  readonly catalogId: string;
  readonly roomId: string;
  readonly name: string;
  readonly direct: boolean;
};
export type InboxHandoff = {
  readonly handoffId: string;
  readonly roomId: string;
  readonly messageId: string;
  readonly agentName: string;
  readonly status: 'queued' | 'delivered' | 'failed';
  readonly createdAtUnixMs: number;
};
export type InboxIndex = {
  readonly accountId: string;
  readonly rooms: readonly InboxRoom[];
  readonly handoffs: readonly InboxHandoff[];
  readonly limited: boolean;
};
export type InboxFailure = { readonly code: string; readonly retryable: boolean };
export type InboxIndexGateway = {
  readonly read: (accountId: string) => Promise<Result<InboxIndex, InboxFailure>>;
};
export type InboxMatrixSource = {
  readonly accountId: () => string | null;
  readonly isJoined: (roomId: string) => boolean;
  readonly isIgnored: (userId: string) => boolean;
  readonly subscribe: (listener: () => void) => () => void;
};
export type InboxItem = {
  readonly id: string;
  readonly kind: 'mention' | 'reply' | 'direct' | 'handoff';
  readonly room: InboxRoom;
  readonly messageId: string;
  readonly conversation: boolean;
  readonly sender: string;
  readonly text: string;
  readonly timestamp: number;
  readonly read: boolean;
  readonly muted: boolean;
  readonly handoff?: InboxHandoff;
};

export function projectInbox(
  index: InboxIndex,
  messages: MessageGateway,
  source: InboxMatrixSource,
  preferences: WorkspaceIndex,
): {
  readonly items: readonly InboxItem[];
  readonly unavailableRooms: number;
  readonly contacts: ReadonlyMap<string, number>;
} {
  const items = new Map<string, InboxItem>();
  const contacts = new Map<string, number>();
  const remember = (message: RoomMessageSignal, timestamp: number) => {
    if (message.actor.kind === 'agent')
      contacts.set(
        message.actor.agentId,
        Math.max(contacts.get(message.actor.agentId) ?? 0, timestamp),
      );
  };
  let unavailableRooms = 0;
  if (source.accountId() !== index.accountId) return { items: [], unavailableRooms: 0, contacts };
  for (const room of index.rooms) {
    if (!source.isJoined(room.roomId)) continue;
    const result = messages.read(room.roomId);
    if (!result.ok) {
      unavailableRooms += 1;
      continue;
    }
    const byId = new Map(result.value.messages.map((message) => [message.messageId, message]));
    const bySender = new Map(
      result.value.messages
        .filter((message) => message.lifecycle === 'active')
        .map((message) => [message.actor.matrixUserId, message]),
    );
    for (const message of result.value.messages) {
      if (message.lifecycle !== 'active' || source.isIgnored(message.actor.matrixUserId)) continue;
      if (message.actor.matrixUserId === index.accountId) {
        const target = message.relation ? byId.get(message.relation.targetMessageId) : undefined;
        if (target?.lifecycle === 'active' && !source.isIgnored(target.actor.matrixUserId))
          remember(target, message.serverTimestamp);
        for (const mentioned of message.preview?.conversation?.mentions ?? []) {
          const target = bySender.get(mentioned);
          if (target && !source.isIgnored(mentioned)) remember(target, message.serverTimestamp);
        }
      }
      const kind = attentionKind(message, byId, index.accountId, room.direct);
      if (kind === null) continue;
      remember(message, message.serverTimestamp);
      const id = `message:${room.roomId}:${message.messageId}`;
      const cursor = preferences.read.get(room.roomId);
      items.set(id, {
        id,
        kind,
        room,
        messageId: message.messageId,
        conversation: message.preview?.conversation !== undefined,
        sender: message.actor.displayName,
        text: message.preview?.conversation?.text ?? message.preview?.summary ?? '',
        timestamp: message.serverTimestamp,
        read:
          preferences.acknowledged.has(id) ||
          (cursor !== undefined &&
            compareCursors(
              { timestamp: message.serverTimestamp, eventId: message.matrixEventId },
              cursor,
            ) <= 0),
        muted: preferences.muted.has(room.roomId),
      });
    }
  }
  const rooms = new Map(index.rooms.map((room) => [room.roomId, room]));
  for (const handoff of index.handoffs) {
    const room = rooms.get(handoff.roomId);
    if (!room || !source.isJoined(room.roomId)) continue;
    const id = `handoff:${handoff.handoffId}:${handoff.status}`;
    items.set(id, {
      id,
      kind: 'handoff',
      room,
      messageId: handoff.messageId,
      conversation: false,
      sender: handoff.agentName,
      text: '',
      timestamp: handoff.createdAtUnixMs,
      read: preferences.acknowledged.has(id),
      muted: preferences.muted.has(room.roomId),
      handoff,
    });
  }
  return {
    items: [...items.values()].toSorted(
      (left, right) => right.timestamp - left.timestamp || left.id.localeCompare(right.id),
    ),
    unavailableRooms,
    contacts,
  };
}

export function notificationAllowed(
  item: InboxItem,
  preferences: WorkspaceIndex,
  now: number,
): boolean {
  return (
    !item.read &&
    !item.muted &&
    preferences.doNotDisturbUntil !== null &&
    preferences.doNotDisturbUntil <= now
  );
}

function attentionKind(
  message: RoomMessageSignal,
  byId: ReadonlyMap<string, RoomMessageSignal>,
  self: string,
  direct: boolean,
): InboxItem['kind'] | null {
  if (message.actor.matrixUserId === self) return null;
  if (message.preview?.conversation?.mentions.includes(self) === true) return 'mention';
  if (message.relation && byId.get(message.relation.targetMessageId)?.actor.matrixUserId === self)
    return 'reply';
  return direct ? 'direct' : null;
}
