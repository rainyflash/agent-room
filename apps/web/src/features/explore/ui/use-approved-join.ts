import { useEffect, useRef } from 'react';

import {
  approvedRequests,
  type ExploreFailure,
  type ExplorePage,
  type ExploreRoomDetail,
} from '@/features/explore/domain/explore';
import type { Result } from '@/shared/result';

import type { ExploreActions } from './use-explore-actions';

/**
 * 申请被批准以后，停在发现页上的人自动进房间，不用再点一次“接受邀请”（`specs/public-rooms/design.md`
 * “加入方式”）。在等批准时目录和详情每 10 秒重读一次（`explore-queries`），看到批准了就照“接受邀请”那一套
 * 替他加入、进房间。离开了发现页的，邀请照旧在“房间”页上等他点。
 */
export function useApprovedJoin({
  detail,
  onJoin,
  pages,
}: {
  readonly detail: Result<ExploreRoomDetail, ExploreFailure> | undefined;
  readonly onJoin: ExploreActions['onJoin'] | undefined;
  readonly pages: readonly Result<ExplorePage, ExploreFailure>[] | undefined;
}): void {
  const waiting = useRef(new Set<string>());
  useEffect(() => {
    if (onJoin === undefined) return;
    const rooms = [
      ...(detail?.ok === true ? [detail.value] : []),
      ...(pages ?? []).flatMap((page) => (page.ok ? page.value.rooms : [])),
    ];
    for (const room of approvedRequests(waiting.current, rooms)) onJoin(room);
  }, [detail, onJoin, pages]);
}
