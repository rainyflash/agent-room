import type { PrivateRoom } from '@/features/private-rooms/domain/private-room';
import type { PublicRoomSummary } from '@/features/room-directory/domain/public-room-directory';
import type { Result } from '@/shared/result';

import type { AgentDirectoryFailure, AgentRoom } from './agent-directory';

/** “带进房间”里的一个房间。私人房间里你没有“自动发言”时，它进去只能看、说不了话。 */
export type BringCandidate = {
  readonly catalogId: string;
  readonly kind: AgentRoom['kind'];
  readonly name: string;
  readonly viewOnly: boolean;
};

/**
 * 你能带 Agent 进的房间（`specs/agent-identity/design.md`“从‘我的 Agent’带进房间”）：公共大厅都行，
 * 私人房间要你已经加入、能发言，和 Agent 自己进来的规则一样。它已经在的房间不列。私人房间在前：
 * 带进自己的房间是更常见的事。
 */
export function bringCandidates({
  current,
  principalId,
  privateRooms,
  publicRooms,
}: {
  readonly current: readonly AgentRoom[];
  readonly principalId: string | undefined;
  readonly privateRooms: readonly PrivateRoom[];
  readonly publicRooms: readonly PublicRoomSummary[];
}): readonly BringCandidate[] {
  const inside = new Set(current.map((room) => room.catalogId));
  const candidates: BringCandidate[] = [];
  for (const room of privateRooms) {
    if (room.status !== 'active' || inside.has(room.catalogId)) continue;
    const member = room.members.find((entry) => entry.principalId === principalId);
    if (member?.status !== 'joined') continue;
    const capabilities = member.permissions.capabilities;
    if (!capabilities.includes('speak')) continue;
    candidates.push({
      catalogId: room.catalogId,
      kind: 'private_room',
      name: room.name,
      viewOnly: !capabilities.includes('automate'),
    });
  }
  for (const room of publicRooms) {
    if (inside.has(room.catalogId)) continue;
    candidates.push({
      catalogId: room.catalogId,
      kind: 'public_lobby',
      name: room.name,
      viewOnly: false,
    });
  }
  return candidates;
}

/** 房间正在准备时服务器回的错误码，带着过多久再试。 */
export const AGENT_ROOM_BUSY = 'agent_room.busy';

const BUSY_ATTEMPTS = 3;
const BUSY_WAIT_LIMIT_MS = 10_000;

/**
 * 带进房间。公共大厅正在开新的分片、或者刚选好的分片满了时，服务器回 `agent_room.busy` 带
 * `Retry-After`：照它说的等一会儿再试，最多试三次，每次最多等 10 秒。
 */
export async function bringWhenReady(
  bring: () => Promise<Result<number, AgentDirectoryFailure>>,
  sleep: (milliseconds: number) => Promise<void>,
): Promise<Result<number, AgentDirectoryFailure>> {
  let result = await bring();
  for (let attempt = 1; attempt < BUSY_ATTEMPTS; attempt += 1) {
    if (result.ok || result.error.code !== AGENT_ROOM_BUSY) return result;
    const seconds = result.error.retryAfterSeconds ?? 1;
    await sleep(Math.min(Math.max(seconds, 1) * 1_000, BUSY_WAIT_LIMIT_MS));
    result = await bring();
  }
  return result;
}
