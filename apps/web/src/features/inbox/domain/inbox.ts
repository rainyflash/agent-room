import type { MessageGateway, RoomMessageSignal } from '@/features/messages/domain/message';
import {
  compareCursors,
  type ReadingCursor,
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
/**
 * 一个房间有多少没读的、有没有人点名你（“房间”页的卡片，`specs/workspace-tidy/design.md`）。
 * 没读的按收件箱那份“读到哪儿”算，只数别人发的话。
 */
export type RoomUnread = {
  readonly catalogId: string;
  /** 读到的位置以后别人说了几句；还没有读到的位置（从没打开过）时不数，是 null。 */
  readonly count: number | null;
  /** 本机的消息没接到读到的位置：真实的只会更多。 */
  readonly more: boolean;
  /** 有没读的点名你或 @所有人。 */
  readonly mentioned: boolean;
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
  /** 按 Matrix 房间号；公开大厅的每个分片各一份。 */
  readonly rooms: ReadonlyMap<string, RoomUnread>;
} {
  const items = new Map<string, InboxItem>();
  const contacts = new Map<string, number>();
  const unread = new Map<string, RoomUnread>();
  const remember = (message: RoomMessageSignal, timestamp: number) => {
    if (message.actor.kind === 'agent')
      contacts.set(
        message.actor.agentId,
        Math.max(contacts.get(message.actor.agentId) ?? 0, timestamp),
      );
  };
  let unavailableRooms = 0;
  if (source.accountId() !== index.accountId)
    return { items: [], unavailableRooms: 0, contacts, rooms: unread };
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
    const cursor = preferences.read.get(room.roomId);
    let mentioned = false;
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
      const read =
        preferences.acknowledged.has(id) ||
        (cursor !== undefined && compareCursors(position(message), cursor) <= 0);
      if (kind === 'mention' && !read) mentioned = true;
      items.set(id, {
        id,
        kind,
        room,
        messageId: message.messageId,
        conversation: message.preview?.conversation !== undefined,
        sender: message.actor.displayName,
        text: message.preview?.conversation?.text ?? message.preview?.summary ?? '',
        timestamp: message.serverTimestamp,
        read,
        muted: preferences.muted.has(room.roomId),
      });
    }
    unread.set(room.roomId, {
      catalogId: room.catalogId,
      ...unreadCount(result.value.messages, cursor, index.accountId, source),
      mentioned,
    });
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
    rooms: unread,
  };
}

function position(message: RoomMessageSignal) {
  return { timestamp: message.serverTimestamp, eventId: message.matrixEventId };
}

/**
 * 读到的位置以后别人说的话有几句。本机只同步了最近的一段：一条读到的位置以前（含）的消息
 * 都没有，就说明没接上，真实的只会更多。
 */
function unreadCount(
  messages: readonly RoomMessageSignal[],
  cursor: ReadingCursor | undefined,
  self: string,
  source: InboxMatrixSource,
): Pick<RoomUnread, 'count' | 'more'> {
  if (cursor === undefined) return { count: null, more: false };
  let count = 0;
  let reached = false;
  for (const message of messages) {
    if (compareCursors(position(message), cursor) <= 0) {
      reached = true;
      continue;
    }
    if (
      message.lifecycle === 'active' &&
      message.preview?.conversation !== undefined &&
      message.actor.matrixUserId !== self &&
      !source.isIgnored(message.actor.matrixUserId)
    )
      count += 1;
  }
  return { count, more: count > 0 && !reached };
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

const notificationTextLimit = 140;

export type SystemNotificationSummary = {
  readonly count: number;
  /** Room name when every fresh item comes from one room. */
  readonly room: string | null;
  /** Sender and text of the only fresh item; null when there are several. */
  readonly sender: string | null;
  readonly text: string;
};

/** What an operating-system notification may say: one item quotes it briefly, more only count. */
export function systemNotificationSummary(fresh: readonly InboxItem[]): SystemNotificationSummary {
  const rooms = new Set(fresh.map((item) => item.room.name).filter((name) => name.length > 0));
  const only = fresh.length === 1 ? fresh[0] : undefined;
  const characters = Array.from(only?.text ?? '');
  const text =
    characters.length > notificationTextLimit
      ? `${characters.slice(0, notificationTextLimit - 1).join('')}…`
      : characters.join('');
  return {
    count: fresh.length,
    room: rooms.size === 1 ? ([...rooms][0] ?? null) : null,
    sender: only?.sender ?? null,
    text,
  };
}

function attentionKind(
  message: RoomMessageSignal,
  byId: ReadonlyMap<string, RoomMessageSignal>,
  self: string,
  direct: boolean,
): InboxItem['kind'] | null {
  if (message.actor.matrixUserId === self) return null;
  if (message.preview?.conversation?.mentions.includes(self) === true) return 'mention';
  // 私人房间里的 @所有人：收的时候已经只认加密消息上的。
  if (message.preview?.mentionsEveryone === true) return 'mention';
  if (message.relation && byId.get(message.relation.targetMessageId)?.actor.matrixUserId === self)
    return 'reply';
  return direct ? 'direct' : null;
}
