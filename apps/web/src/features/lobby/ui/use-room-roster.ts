import { useMemo } from 'react';

import { useAppServices } from '@/app/app-services';
import { useWakeableAgentIds } from '@/features/desktop/ui/use-wakeable-agents';
import { usePrivateRoomPeople } from '@/features/private-rooms/data/private-room-queries';
import type { PrivateRoom } from '@/features/private-rooms/domain/private-room';
import {
  roomPeople,
  type RoomPeople,
  type SpokenPerson,
} from '@/features/private-rooms/domain/room-people';

import { attendanceCounts } from '../domain/agent-attendance';
import type { LobbyAgent } from '../domain/lobby';
import { useAgentOwners, useAgentOwnerText } from './agent-owners';

export type RoomRosterData = {
  /** 表格里列着的 Agent 数：归档的不算，离线的算。顶栏和“Agent”标签上是同一个数。 */
  readonly agentCount: number;
  readonly people: RoomPeople;
  readonly wakeableAgentIds: ReadonlySet<string>;
};

/**
 * 房间页右栏要的数：房间里的人、此刻叫得醒的 Agent、Agent 的个数。顶栏那一行小字也用它，
 * 两处的数才对得上。
 *
 * `room` 是你的房间列表里的这一间：公共大厅不在列表里，没有成员名单，人就只列说过话的。
 */
export function useRoomRoster({
  agents,
  catalogId,
  observedAtUnixMs,
  principalId,
  room,
  roomId,
  spoken,
}: {
  readonly agents: readonly LobbyAgent[];
  readonly catalogId: string;
  readonly observedAtUnixMs: number;
  readonly principalId: string | null;
  readonly room: PrivateRoom | undefined;
  readonly roomId: string;
  readonly spoken: readonly SpokenPerson[];
}): RoomRosterData {
  const { privateRooms } = useAppServices();
  const names = usePrivateRoomPeople(
    privateRooms,
    catalogId,
    principalId !== null && room !== undefined,
  );
  const listedNames = names.data?.ok === true ? names.data.value : null;
  const people = useMemo(
    () => roomPeople(room, listedNames, principalId, spoken),
    [listedNames, principalId, room, spoken],
  );
  // 叫得醒只知道你自己的：这间房里没有你的 Agent 就不去问后台回复的登记。
  const owners = useAgentOwners(agents.map((agent) => agent.agentId));
  const { mine } = useAgentOwnerText();
  const hasMine = [...owners.values()].some(mine);
  const wakeableAgentIds = useWakeableAgentIds(roomId, principalId, hasMine);
  const attendance = attendanceCounts(agents, observedAtUnixMs);
  return {
    agentCount: attendance.present + attendance.reconnecting + attendance.away,
    people,
    wakeableAgentIds,
  };
}
