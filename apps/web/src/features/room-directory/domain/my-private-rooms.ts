import type {
  PrivateRoom,
  PrivateRoomOrigin,
  PrivateRoomPerson,
} from '@/features/private-rooms/domain/private-room';

export type GroupedPrivateRooms = {
  /** 有人邀请了你、你还没答复的房间。 */
  readonly invitations: readonly PrivateRoom[];
  readonly joined: readonly PrivateRoom[];
  /** 归档了的房间：谁都不能再说话，之前的消息还能看。只算你加入过的，拒绝过、被移出的不算。 */
  readonly archived: readonly PrivateRoom[];
};

/** 按你在房间里的身份分开私人房间；还在用的房间里，受邀的和加入的各归一处。 */
export function groupMyPrivateRooms(
  rooms: readonly PrivateRoom[],
  principalId: string | undefined,
): GroupedPrivateRooms {
  const membership = (room: PrivateRoom) =>
    room.members.find((member) => member.principalId === principalId)?.status;
  const active = rooms.filter((room) => room.status === 'active');
  return {
    invitations: active.filter((room) => membership(room) === 'invited'),
    joined: active.filter((room) => membership(room) === 'joined'),
    archived: rooms.filter((room) => room.status === 'archived' && membership(room) === 'joined'),
  };
}

export type RoomsByOwner = {
  /** 房主是你的：你自己建的，和你的 Agent 替你建的（房主是人，不是 Agent）。 */
  readonly created: readonly PrivateRoom[];
  /** 房主是别人的：别人建的，网络 Agent 建的也在这里。 */
  readonly others: readonly PrivateRoom[];
};

/** 按房主分开：房主是你的归“你建的”，别的都归“你加入的”。还不知道你是谁时都算别人的。 */
export function splitByOwner(
  rooms: readonly PrivateRoom[],
  principalId: string | undefined,
): RoomsByOwner {
  const mine = (room: PrivateRoom) =>
    principalId !== undefined && room.ownerPrincipalId === principalId;
  return {
    created: rooms.filter(mine),
    others: rooms.filter((room) => !mine(room)),
  };
}

/** 别人房间的房主：一个人（昵称），或者一个网络 Agent（名字查不到时为空）。 */
export type RoomOwner =
  | { readonly kind: 'person'; readonly name: string }
  | { readonly kind: 'network_agent'; readonly name: string | null };

/**
 * 房主是谁。先要知道房间是谁建的：房主是网络 Agent 时写建房的那个 Agent 的名字，不写替它记账的合成主体；
 * 是人时用成员名单里他的昵称。哪一样还没问到、没问成，或者名单里没有房主，就先不写（null），房间照样列出来。
 */
export function roomOwnerOf(
  room: PrivateRoom,
  origin: PrivateRoomOrigin | undefined,
  people: readonly PrivateRoomPerson[] | undefined,
): RoomOwner | null {
  if (origin === undefined) return null;
  if (origin.ownerIsNetworkAgent) {
    return { kind: 'network_agent', name: shown(origin.createdByAgent?.displayName) };
  }
  const name = shown(
    people?.find((person) => person.principalId === room.ownerPrincipalId)?.displayName,
  );
  return name === null ? null : { kind: 'person', name };
}

function shown(name: string | null | undefined): string | null {
  const trimmed = name?.trim();
  return trimmed === undefined || trimmed === '' ? null : trimmed;
}
