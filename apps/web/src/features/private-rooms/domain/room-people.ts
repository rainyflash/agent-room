import { isRoomAdmin, type PrivateRoom, type PrivateRoomPerson } from './private-room';

/** 房间页“人”那一栏的一行。 */
export type RoomPerson = {
  /** 这一行自己的标识：有成员名单时是账户 ID，没有时是聊天账户。 */
  readonly key: string;
  readonly displayName: string;
  readonly self: boolean;
  readonly role: 'owner' | 'admin' | null;
  /** 账户 ID：数他带了几个 Agent 用。没有成员名单的房间里不知道。 */
  readonly principalId: string | null;
};

export type RoomPeople = {
  readonly people: readonly RoomPerson[];
  /** 是不是照成员名单列的。不是的话列的只是说过话的人。 */
  readonly listed: boolean;
};

/** 说过话的人：公共大厅没有成员名单，只知道这些。 */
export type SpokenPerson = {
  readonly matrixUserId: string;
  readonly displayName: string;
  readonly isSelf: boolean;
};

const roleOrder = { owner: 0, admin: 1, none: 2 } as const;

/**
 * 房间里的人。
 * - 有成员名单的房间（私人房间、公开房间）：已加入的成员，房主在前、管理员其次，别的按名字排。
 *   没有昵称的成员不列：网络 Agent 当房主时那个替它记账的主体不是人。
 * - 没有名单的（公共大厅），或者名单还没读到：说过话的人和你自己。
 */
export function roomPeople(
  room: PrivateRoom | undefined,
  names: readonly PrivateRoomPerson[] | null,
  selfPrincipalId: string | null,
  spoken: readonly SpokenPerson[],
): RoomPeople {
  if (room === undefined || names === null) {
    return {
      listed: false,
      people: spoken.map((person) => ({
        key: person.matrixUserId,
        displayName: person.displayName,
        self: person.isSelf,
        role: null,
        principalId: null,
      })),
    };
  }
  const nameOf = new Map(names.map((person) => [person.principalId, person.displayName.trim()]));
  const people = room.members.flatMap((member): RoomPerson[] => {
    const displayName = nameOf.get(member.principalId);
    if (member.status !== 'joined' || displayName === undefined || displayName === '') return [];
    return [
      {
        key: member.principalId,
        displayName,
        self: member.principalId === selfPrincipalId,
        role:
          member.principalId === room.ownerPrincipalId
            ? 'owner'
            : isRoomAdmin(room, member)
              ? 'admin'
              : null,
        principalId: member.principalId,
      },
    ];
  });
  return {
    listed: true,
    people: people.toSorted(
      (a, b) =>
        roleOrder[a.role ?? 'none'] - roleOrder[b.role ?? 'none'] ||
        a.displayName.localeCompare(b.displayName),
    ),
  };
}
