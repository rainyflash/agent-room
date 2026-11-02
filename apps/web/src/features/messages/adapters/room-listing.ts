import type { Room } from 'matrix-js-sdk';
import { z } from 'zod';

import { DIRECTION_FORWARD } from '@/shared/matrix/matrix-sdk-enums';

/** 房间第一次公开的那一刻，由应用服务写（`specs/public-rooms/design.md`）。 */
export const roomListingEventType = 'io.github.rainyflash.agentroom.room.listing.v1';

const listingSchema = z.object({ firstListedAt: z.iso.datetime({ offset: true }) });

/**
 * 这个人此刻的成员事件是不是在房间第一次公开以后。房间没公开过返回 false。
 *
 * 加密的私密房间改成公开以后，这一刻以后进来的人不该读到以前私下说的话。房间密钥里没有时间，
 * 分不清哪个会话是公开以前建的，同一个会话还可能跨过这一刻接着用，所以一刀切：这一刻以后进来的人，
 * 不给他们重发房间密钥，他们自己也不去要（和 matrix-adapter 的 `room_listing` 一样）。
 *
 * 宁可少给，不多给：状态读不出来当成一开始就公开了；成员事件改过（比如改了名字）按最新那条算，
 * 看不出时间的也算后来的。这条状态只会让重发变少，所以不核对是谁写的。
 */
export function joinedAfterListing(room: Room, userId: string): boolean {
  const listing = room
    .getLiveTimeline()
    .getState(DIRECTION_FORWARD)
    ?.getStateEvents(roomListingEventType, '');
  if (listing === null || listing === undefined) return false;
  const parsed = listingSchema.safeParse(listing.getContent());
  const listedAt = parsed.success ? Date.parse(parsed.data.firstListedAt) : 0;
  const joinedAt = room.getMember(userId)?.events.member?.getTs();
  return joinedAt === undefined || joinedAt >= listedAt;
}
