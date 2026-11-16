import { QueryClient, QueryObserver } from '@tanstack/react-query';
import { describe, expect, it, vi } from 'vitest';

import {
  privateRoomAgentKnocksQueryKey,
  privateRoomListQueryKey,
  refreshRoomsAfterClosing,
} from './private-room-queries';

const closed = '018f2a00-0000-7000-8000-000000000001';
const other = '018f2a00-0000-7000-8000-000000000002';

function watch(client: QueryClient, queryKey: readonly unknown[]) {
  const queryFn = vi.fn(() => Promise.resolve('fresh'));
  const observer = new QueryObserver(client, { queryFn, queryKey, staleTime: Infinity });
  const unsubscribe = observer.subscribe(() => undefined);
  return { queryFn, unsubscribe };
}

describe('refreshRoomsAfterClosing', () => {
  it('重读房间列表和别的房间的敲门，归档、退出的这一间的敲门不重问', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const list = watch(client, [...privateRoomListQueryKey, 'principal']);
    const closedKnocks = watch(client, privateRoomAgentKnocksQueryKey(closed));
    const otherKnocks = watch(client, privateRoomAgentKnocksQueryKey(other));
    await vi.waitFor(() => {
      expect(list.queryFn).toHaveBeenCalledTimes(1);
      expect(closedKnocks.queryFn).toHaveBeenCalledTimes(1);
      expect(otherKnocks.queryFn).toHaveBeenCalledTimes(1);
    });

    await refreshRoomsAfterClosing(client, closed);

    expect(list.queryFn).toHaveBeenCalledTimes(2);
    expect(otherKnocks.queryFn).toHaveBeenCalledTimes(2);
    expect(closedKnocks.queryFn).toHaveBeenCalledTimes(1);
    for (const query of [list, closedKnocks, otherKnocks]) query.unsubscribe();
    client.clear();
  });
});
