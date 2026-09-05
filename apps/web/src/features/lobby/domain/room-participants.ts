import type { LobbyRoom } from './lobby';
import type { RoomMessageSignal } from '@/features/messages/domain/message';

export type RoomHuman = {
  readonly matrixUserId: string;
  readonly displayName: string;
  readonly isSelf: boolean;
};
export type RoomIdentity = { readonly matrixUserId: string; readonly displayName: string };

export function roomHumans(
  room: LobbyRoom,
  messages: readonly RoomMessageSignal[],
  identity: RoomIdentity | null,
): readonly RoomHuman[] {
  const joined = new Set(room.joinedMemberIds ?? []);
  const agents = new Set(room.agents.map((agent) => agent.matrixUserId));
  const humans = new Map<string, RoomHuman>();
  for (const message of messages.toSorted((a, b) => a.serverTimestamp - b.serverTimestamp)) {
    const actor = message.actor;
    if (
      message.roomId !== room.roomId ||
      actor.kind !== 'human' ||
      !joined.has(actor.matrixUserId) ||
      agents.has(actor.matrixUserId)
    )
      continue;
    humans.set(actor.matrixUserId, {
      matrixUserId: actor.matrixUserId,
      displayName: actor.displayName,
      isSelf: false,
    });
  }
  if (
    identity !== null &&
    (room.joinedMemberIds === undefined || joined.has(identity.matrixUserId))
  ) {
    humans.set(identity.matrixUserId, { ...identity, isSelf: true });
  }
  return Object.freeze(
    [...humans.values()].toSorted((a, b) => a.matrixUserId.localeCompare(b.matrixUserId)),
  );
}
