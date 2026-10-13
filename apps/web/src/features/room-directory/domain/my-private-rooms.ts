import type { PrivateRoom } from '@/features/private-rooms/domain/private-room';

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
