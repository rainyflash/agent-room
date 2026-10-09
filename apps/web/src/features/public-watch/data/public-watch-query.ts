import { queryOptions, useQuery } from '@tanstack/react-query';

import {
  isFinalWatchFailure,
  type PublicWatchFailure,
  type PublicWatchGateway,
} from '@/features/public-watch/domain/public-watch';

/** 页面在前台时隔这么久问一次；服务端的快照 3 秒一换，暂时读不到时也请网页隔 5 秒再问。 */
export const publicWatchPollMs = 5_000;

/** 读不到时抛出它：react-query 照旧留着上一份快照，页面接着显示，同时说正在重试。 */
export class PublicWatchError extends Error {
  readonly failure: PublicWatchFailure;

  constructor(failure: PublicWatchFailure) {
    super(failure.code);
    this.name = 'PublicWatchError';
    this.failure = failure;
  }
}

export function publicWatchQueryOptions(gateway: PublicWatchGateway, slug: string) {
  return queryOptions({
    networkMode: 'always',
    queryFn: async () => {
      const result = await gateway.read(slug);
      if (!result.ok) throw new PublicWatchError(result.error);
      return result.value;
    },
    queryKey: ['control-plane', 'public-watch', slug] as const,
    // 切到后台就停；没开放、没有这个大厅时再问也一样，也停。
    refetchInterval: (query) =>
      query.state.error instanceof PublicWatchError &&
      isFinalWatchFailure(query.state.error.failure)
        ? false
        : publicWatchPollMs,
    refetchIntervalInBackground: false,
    refetchOnWindowFocus: 'always',
    retry: false,
  });
}

export function usePublicWatch(gateway: PublicWatchGateway, slug: string) {
  return useQuery(publicWatchQueryOptions(gateway, slug));
}
