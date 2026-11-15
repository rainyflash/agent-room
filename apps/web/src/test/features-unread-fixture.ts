import { conversationFixture } from '@/features/conversation/testing/conversation-fixture';
import { InboxStore } from '@/features/inbox/application/inbox-store';
import type { InboxRoom } from '@/features/inbox/domain/inbox';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { PersonalWorkspaceStore } from '@/features/personal-workspace/application/personal-workspace-store';
import {
  changeWorkspace,
  emptyWorkspace,
  type WorkspaceDocument,
} from '@/features/personal-workspace/domain/workspace-document';
import { ok } from '@/shared/result';

/** 子房间：收件箱里有它、读到的位置停在第一句，后面几句没读；`muted` 的设了免打扰。 */
export type FeaturesUnreadSubRoom = {
  readonly room: InboxRoom;
  readonly unread: number;
  readonly mentionsYou?: boolean;
  readonly muted?: boolean;
};

/**
 * “房间”页夹具（`lobby-scene.html?features=1&unread`）的收件箱：
 * Design studio 读到一半、后面三条里有一条点名你；Engineering room 积了 120 条；
 * 公开大厅的分片里有人点名你。`subRooms` 是加 `&subRooms` 时你在的几间子房间。
 */
export function featuresUnreadInbox(
  accountId: string,
  rooms: readonly InboxRoom[],
  subRooms: readonly FeaturesUnreadSubRoom[] = [],
): { readonly inbox: InboxStore; readonly personal: PersonalWorkspaceStore } | null {
  const [design, engineering, lobby] = rooms;
  if (design === undefined || engineering === undefined || lobby === undefined) return null;
  const start = Date.now() - 60 * 60_000;
  const said = (room: InboxRoom, id: string, minute: number, mentions: readonly string[] = []) =>
    conversationFixture(`${room.roomId}-${id}`, {
      roomId: room.roomId,
      matrixEventId: `$${id}-${room.catalogId}`,
      serverTimestamp: start + minute * 60_000,
      preview: {
        title: 'Conversation',
        summary: 'Hello',
        conversation: { text: 'Hello', mentions },
        contentType: 'text/plain',
        riskFlags: [],
        sensitivity: 'normal',
      },
    });
  const designSeen = said(design, 'seen', 0);
  const engineeringSeen = said(engineering, 'seen', 0);
  const messages = new Map<string, readonly RoomMessageSignal[]>([
    [
      design.roomId,
      [
        designSeen,
        said(design, 'one', 1),
        said(design, 'two', 2, [accountId]),
        said(design, 'three', 3),
      ],
    ],
    [
      engineering.roomId,
      [
        engineeringSeen,
        ...Array.from({ length: 120 }, (_, index) =>
          said(engineering, `n${String(index)}`, 1 + index / 10),
        ),
      ],
    ],
    [lobby.roomId, [said(lobby, 'hello', 5), said(lobby, 'ping', 6, [accountId])]],
  ]);
  const subRoomSeen = subRooms.map(({ room, unread, mentionsYou = false }) => {
    const seen = said(room, 'seen', 0);
    messages.set(room.roomId, [
      seen,
      ...Array.from({ length: unread }, (_, index) =>
        said(room, `s${String(index)}`, 1 + index, mentionsYou && index === 0 ? [accountId] : []),
      ),
    ]);
    return seen;
  });
  let document: WorkspaceDocument = emptyWorkspace;
  const remember = (change: Parameters<typeof changeWorkspace>[1]) => {
    const changed = changeWorkspace(document, change, 'fixture-device');
    if (changed.ok) document = changed.value;
  };
  for (const seen of [designSeen, engineeringSeen, ...subRoomSeen]) {
    remember({
      kind: 'read',
      id: seen.roomId,
      value: { eventId: seen.matrixEventId, timestamp: seen.serverTimestamp },
    });
  }
  for (const { room, muted = false } of subRooms)
    if (muted) remember({ kind: 'mute', id: room.roomId, value: true });
  let cached = document;
  const personal = new PersonalWorkspaceStore(
    {
      scope: () => ({ accountId, writerId: 'fixture-device' }),
      read: () => Promise.resolve(ok(document)),
      write: (_scope, next) => {
        document = next;
        return Promise.resolve(ok(undefined));
      },
      subscribe: () => () => undefined,
    },
    {
      read: () => ok(cached),
      write: (_account, next) => {
        cached = next;
        return ok(undefined);
      },
    },
  );
  const indexRooms = [...rooms, ...subRooms.map(({ room }) => room)];
  const inbox = new InboxStore(
    {
      read: () =>
        Promise.resolve(ok({ accountId, rooms: indexRooms, handoffs: [], limited: false })),
    },
    {
      accountId: () => accountId,
      isJoined: (roomId) => messages.has(roomId),
      isIgnored: () => false,
      subscribe: () => () => undefined,
    },
    {
      read: (roomId) =>
        ok({
          roomId,
          messages: messages.get(roomId) ?? [],
          observedAtUnixMs: Date.now(),
          readOnlyFederatedEvents: [],
        }),
      subscribe: () => () => undefined,
    },
    personal,
  );
  return { inbox, personal };
}
