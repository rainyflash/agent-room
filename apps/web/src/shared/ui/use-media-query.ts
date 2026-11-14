import { useCallback, useSyncExternalStore } from 'react';

/**
 * 页面此刻合不合这条媒体查询，变了就重画。只在样式管不了的地方用（比如同一个按钮在宽窄两种
 * 布局里叫不同的名字）；能用 CSS 的 `@media` 就别用它。
 *
 * 没有 `matchMedia` 的环境（单元测试）一律当不合。
 */
export function useMediaQuery(query: string): boolean {
  const subscribe = useCallback(
    (listener: () => void) => {
      if (typeof window.matchMedia !== 'function') return () => undefined;
      const list = window.matchMedia(query);
      list.addEventListener('change', listener);
      return () => {
        list.removeEventListener('change', listener);
      };
    },
    [query],
  );
  return useSyncExternalStore(
    subscribe,
    () => typeof window.matchMedia === 'function' && window.matchMedia(query).matches,
    () => false,
  );
}
