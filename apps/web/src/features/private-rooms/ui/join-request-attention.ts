import { useAppServices } from '@/app/app-services';
import {
  usePrivateRoomJoinRequests,
  usePrivateRoomListing,
} from '@/features/private-rooms/data/private-room-queries';
import { allows, memberFor, type PrivateRoom } from '@/features/private-rooms/domain/private-room';

/** 房间页上隔多久再问一次有没有新申请，和敲门一样。 */
const JOIN_REQUEST_POLL_MS = 10_000;

/**
 * 房间页齿轮上的点（`specs/public-rooms/design.md`“网页”）：你管着这间公开房间、有人在等批准时，
 * 返回有几个。不是管理者、房间没公开就不问，返回 0。
 */
export function usePendingJoinRequestCount(
  rooms: readonly PrivateRoom[],
  catalogId: string,
  principalId: string | null,
): number {
  const { privateRoomListings, publicRoomJoins } = useAppServices();
  const room = rooms.find(
    (candidate) => candidate.catalogId === catalogId && candidate.status === 'active',
  );
  const manages =
    room !== undefined &&
    principalId !== null &&
    (room.ownerPrincipalId === principalId || allows(memberFor(room, principalId), 'manage'));
  const listing = usePrivateRoomListing(privateRoomListings, catalogId, manages);
  const listed = listing.data?.ok === true && listing.data.value.listed;
  const requests = usePrivateRoomJoinRequests(publicRoomJoins, catalogId, {
    enabled: manages && listed,
    intervalMs: JOIN_REQUEST_POLL_MS,
  });
  return manages && listed && requests.data?.ok === true ? requests.data.value.length : 0;
}
